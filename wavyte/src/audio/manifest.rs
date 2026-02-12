use crate::assets::media;
use crate::assets::media::{VideoSourceInfo, decode_audio_f32_stereo, probe_video};
use crate::assets::store::normalize_rel_path;
use crate::audio::mix::frame_to_sample;
use crate::eval::context::NodeTimeCtx;
use crate::eval::properties::{PropertyEvalScratch, PropertyValues, eval_expr_program_frame};
use crate::eval::time::compute_node_time_ctxs;
use crate::eval::visibility::{VisibilityState, compute_visibility};
use crate::expression::program::ExprProgram;
use crate::foundation::core::FrameRange;
use crate::foundation::error::{WavyteError, WavyteResult};
use crate::foundation::ids::{AssetIdx, NodeIdx};
use crate::normalize::intern::StringInterner;
use crate::normalize::ir::{AssetIR, CollectionModeIR, CompositionIR, NodeKindIR};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
/// One scheduled audio contribution in timeline sample space.
pub(crate) struct AudioSegment {
    pub(crate) timeline_start_sample: u64,
    pub(crate) timeline_end_sample: u64,
    pub(crate) source_start_sec: f64,
    pub(crate) source_end_sec: Option<f64>,
    pub(crate) playback_rate: f64,
    pub(crate) volume: f32,
    pub(crate) fade_in_sec: f64,
    pub(crate) fade_out_sec: f64,
    pub(crate) source_sample_rate: u32,
    pub(crate) source_channels: u16,
    pub(crate) source_interleaved_f32: Arc<Vec<f32>>,
}

#[derive(Clone, Debug)]
/// Audio rendering plan for a timeline frame range.
pub(crate) struct AudioManifest {
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
    pub(crate) total_samples: u64,
    pub(crate) segments: Vec<AudioSegment>,
}

#[derive(Clone, Copy, Debug)]
struct SegState {
    start_global_frame: u64,
    start_local_frame: u64,
    last_global_frame: u64,
    last_local_frame: u64,
}

#[derive(Clone, Debug)]
struct CachedPcm {
    sample_rate: u32,
    channels: u16,
    data: Arc<Vec<f32>>,
}

/// Build audio mixing manifest for the given timeline range.
///
/// This runs outside the per-frame render hot loop. For `Switch` nodes, v0.3 enforces that the
/// active child is constant over the entire render range if that switch contains any audio-capable
/// descendants; otherwise a validation error is returned.
pub(crate) fn build_audio_manifest(
    ir: &CompositionIR,
    interner: &StringInterner,
    assets_root: &Path,
    expr_program: &ExprProgram,
    range: FrameRange,
) -> WavyteResult<AudioManifest> {
    if range.is_empty() {
        return Err(WavyteError::validation(
            "audio manifest range must be non-empty",
        ));
    }
    if range.end.0 > ir.duration_frames {
        return Err(WavyteError::validation(
            "audio manifest range must be within composition duration",
        ));
    }

    let sample_rate = media::MIX_SAMPLE_RATE;
    let channels = 2u16;

    let n_nodes = ir.nodes.len();
    let mut time_ctxs = Vec::<NodeTimeCtx>::new();
    let mut props = PropertyValues::new(expr_program);
    let mut props_scratch = PropertyEvalScratch::new();
    let mut vis = VisibilityState::default();

    let mut has_audio_desc = vec![None::<bool>; n_nodes];
    let mut switch_active_seen = vec![None::<NodeIdx>; n_nodes];
    let mut seg_state = vec![None::<SegState>; n_nodes];
    let mut segments = Vec::<AudioSegment>::new();

    let mut audio_cache: Vec<Option<CachedPcm>> = vec![None; ir.assets.len()];
    let mut video_probe_cache: Vec<Option<Arc<VideoSourceInfo>>> = vec![None; ir.assets.len()];

    for gf in range.start.0..range.end.0 {
        compute_node_time_ctxs(ir, gf, &mut time_ctxs);
        eval_expr_program_frame(ir, &time_ctxs, expr_program, &mut props, &mut props_scratch)
            .map_err(|e| WavyteError::evaluation(format!("audio expr eval failed: {e}")))?;
        compute_visibility(ir, &time_ctxs, Some(&props), &mut vis)
            .map_err(|e| WavyteError::evaluation(format!("audio visibility failed: {e}")))?;

        // Switch audio constraint: active child must be constant over the render range if this
        // switch contains any audio-capable descendant.
        for (i, node) in ir.nodes.iter().enumerate() {
            if !vis.node_visible[i] {
                continue;
            }
            let NodeKindIR::Collection { mode, .. } = &node.kind else {
                continue;
            };
            if !matches!(mode, CollectionModeIR::Switch) {
                continue;
            }
            let idx = NodeIdx(i as u32);
            if !node_has_audio_descendant(ir, idx, &mut has_audio_desc) {
                continue;
            }
            let active = vis.switch_active_child[i];
            if let Some(prev) = switch_active_seen[i] {
                if active != Some(prev) {
                    return Err(WavyteError::validation(
                        "audio mixing requires switch.active to be constant over the render range",
                    ));
                }
            } else if let Some(a) = active {
                switch_active_seen[i] = Some(a);
            }
        }

        // Update per-leaf segment state.
        for (i, node) in ir.nodes.iter().enumerate() {
            if !vis.node_visible[i] {
                continue;
            }
            let NodeKindIR::Leaf { asset } = &node.kind else {
                continue;
            };
            if !asset_is_audio_capable(ir, *asset) {
                continue;
            }

            let t = time_ctxs[i];
            let local_frame = t.sample_frame_u64();
            let idx = NodeIdx(i as u32);
            match &mut seg_state[i] {
                Some(st)
                    if st.last_global_frame + 1 == gf && st.last_local_frame + 1 == local_frame =>
                {
                    st.last_global_frame = gf;
                    st.last_local_frame = local_frame;
                }
                slot => {
                    // Close any existing segment and start a new one.
                    if let Some(old) = slot.take() {
                        close_segment(
                            ir,
                            interner,
                            assets_root,
                            range,
                            idx,
                            *asset,
                            old,
                            &mut audio_cache,
                            &mut video_probe_cache,
                            &mut segments,
                        )?;
                    }
                    *slot = Some(SegState {
                        start_global_frame: gf,
                        start_local_frame: local_frame,
                        last_global_frame: gf,
                        last_local_frame: local_frame,
                    });
                }
            }
        }

        // Close segments for nodes that were active in the previous frame but not seen now.
        for (i, st) in seg_state.iter_mut().enumerate() {
            let Some(cur) = st else { continue };
            if cur.last_global_frame == gf {
                continue;
            }

            let idx = NodeIdx(i as u32);
            let node = &ir.nodes[i];
            let NodeKindIR::Leaf { asset } = &node.kind else {
                *st = None;
                continue;
            };
            let old = st.take().unwrap();
            close_segment(
                ir,
                interner,
                assets_root,
                range,
                idx,
                *asset,
                old,
                &mut audio_cache,
                &mut video_probe_cache,
                &mut segments,
            )?;
        }
    }

    // Close any remaining segments.
    for (i, st) in seg_state.into_iter().enumerate() {
        let Some(old) = st else { continue };
        let node = &ir.nodes[i];
        let NodeKindIR::Leaf { asset } = &node.kind else {
            continue;
        };
        close_segment(
            ir,
            interner,
            assets_root,
            range,
            NodeIdx(i as u32),
            *asset,
            old,
            &mut audio_cache,
            &mut video_probe_cache,
            &mut segments,
        )?;
    }

    Ok(AudioManifest {
        sample_rate,
        channels,
        total_samples: frame_to_sample(range.len_frames(), ir.fps, sample_rate),
        segments,
    })
}

fn node_has_audio_descendant(ir: &CompositionIR, idx: NodeIdx, memo: &mut [Option<bool>]) -> bool {
    let i = idx.0 as usize;
    if let Some(v) = memo[i] {
        return v;
    }

    let v = match &ir.nodes[i].kind {
        NodeKindIR::Leaf { asset } => asset_is_audio_capable(ir, *asset),
        NodeKindIR::Collection { children, .. } => children
            .iter()
            .copied()
            .any(|c| node_has_audio_descendant(ir, c, memo)),
        NodeKindIR::CompRef { .. } => false,
    };

    memo[i] = Some(v);
    v
}

fn asset_is_audio_capable(ir: &CompositionIR, a: AssetIdx) -> bool {
    matches!(
        ir.assets.get(a.0 as usize),
        Some(AssetIR::Audio { .. }) | Some(AssetIR::Video { .. })
    )
}

#[allow(clippy::too_many_arguments)]
fn close_segment(
    ir: &CompositionIR,
    interner: &StringInterner,
    assets_root: &Path,
    render_range: FrameRange,
    _node: NodeIdx,
    asset: AssetIdx,
    st: SegState,
    audio_cache: &mut [Option<CachedPcm>],
    video_probe_cache: &mut [Option<Arc<VideoSourceInfo>>],
    out: &mut Vec<AudioSegment>,
) -> WavyteResult<()> {
    let seg_start_frame = st.start_global_frame;
    let seg_end_frame_excl = st.last_global_frame + 1;
    if seg_start_frame >= seg_end_frame_excl {
        return Ok(());
    }

    let (AssetIR::Audio {
        source,
        trim_start_sec,
        trim_end_sec,
        playback_rate,
        volume,
        mute,
        fade_in_sec,
        fade_out_sec,
    }
    | AssetIR::Video {
        source,
        trim_start_sec,
        trim_end_sec,
        playback_rate,
        volume,
        mute,
        fade_in_sec,
        fade_out_sec,
    }) = &ir.assets[asset.0 as usize]
    else {
        return Ok(());
    };

    if *mute || *volume <= 0.0 {
        return Ok(());
    }

    // Probe video-only sources to skip decoding when no audio stream exists.
    if matches!(&ir.assets[asset.0 as usize], AssetIR::Video { .. }) {
        let info = ensure_video_probe(interner, assets_root, *source, asset, video_probe_cache)?;
        if !info.has_audio {
            return Ok(());
        }
    }

    let pcm = ensure_audio_pcm(interner, assets_root, *source, asset, audio_cache)?;
    if pcm.data.is_empty() {
        return Ok(());
    }

    let timeline_start_sample = frame_to_sample(
        seg_start_frame.saturating_sub(render_range.start.0),
        ir.fps,
        media::MIX_SAMPLE_RATE,
    );
    let timeline_end_sample = frame_to_sample(
        seg_end_frame_excl.saturating_sub(render_range.start.0),
        ir.fps,
        media::MIX_SAMPLE_RATE,
    );

    let local_start = st.start_local_frame;
    let local_start_sec = (local_start as f64) * (f64::from(ir.fps.den) / f64::from(ir.fps.num));
    let mut source_start_sec = (*trim_start_sec) + local_start_sec * (*playback_rate);
    if let Some(end) = trim_end_sec {
        source_start_sec = source_start_sec.min(end.max(*trim_start_sec));
    }
    source_start_sec = source_start_sec.max(0.0);

    out.push(AudioSegment {
        timeline_start_sample,
        timeline_end_sample,
        source_start_sec,
        source_end_sec: *trim_end_sec,
        playback_rate: *playback_rate,
        volume: (*volume as f32).max(0.0),
        fade_in_sec: (*fade_in_sec).max(0.0),
        fade_out_sec: (*fade_out_sec).max(0.0),
        source_sample_rate: pcm.sample_rate,
        source_channels: pcm.channels,
        source_interleaved_f32: pcm.data.clone(),
    });

    Ok(())
}

fn ensure_video_probe(
    interner: &StringInterner,
    assets_root: &Path,
    source: crate::normalize::intern::InternId,
    asset: AssetIdx,
    cache: &mut [Option<Arc<VideoSourceInfo>>],
) -> WavyteResult<Arc<VideoSourceInfo>> {
    let i = asset.0 as usize;
    if let Some(info) = cache.get(i).and_then(|x| x.as_ref()).cloned() {
        return Ok(info);
    }
    let path = asset_path(interner, assets_root, source)?;
    let info = probe_video(&path)?;
    let arc = Arc::new(info);
    cache[i] = Some(arc.clone());
    Ok(arc)
}

fn ensure_audio_pcm(
    interner: &StringInterner,
    assets_root: &Path,
    source: crate::normalize::intern::InternId,
    asset: AssetIdx,
    cache: &mut [Option<CachedPcm>],
) -> WavyteResult<CachedPcm> {
    let i = asset.0 as usize;
    if let Some(pcm) = cache.get(i).and_then(|x| x.as_ref()).cloned() {
        return Ok(pcm);
    }
    let path = asset_path(interner, assets_root, source)?;
    let pcm = decode_audio_f32_stereo(&path, media::MIX_SAMPLE_RATE)?;
    let cached = CachedPcm {
        sample_rate: pcm.sample_rate,
        channels: pcm.channels,
        data: Arc::new(pcm.interleaved_f32),
    };
    cache[i] = Some(cached.clone());
    Ok(cached)
}

fn asset_path(
    interner: &StringInterner,
    assets_root: &Path,
    source: crate::normalize::intern::InternId,
) -> WavyteResult<PathBuf> {
    let rel = interner.get(source);
    let norm = normalize_rel_path(rel)?;
    Ok(assets_root.join(Path::new(&norm)))
}
