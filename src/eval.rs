use crate::{
    anim::SampleCtx,
    core::{FrameIndex, FrameRange},
    error::{WavyteError, WavyteResult},
    model::{BlendMode, Clip, Composition, EffectInstance, TransitionSpec},
};

#[derive(Clone, Debug, serde::Serialize)]
pub struct EvaluatedGraph {
    pub frame: FrameIndex,
    pub nodes: Vec<EvaluatedClipNode>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct EvaluatedClipNode {
    pub clip_id: String,
    pub asset: String,
    pub z: i32,
    pub transform: kurbo::Affine,
    pub opacity: f64,
    pub blend: BlendMode,
    pub effects: Vec<ResolvedEffect>,
    pub transition_in: Option<ResolvedTransition>,
    pub transition_out: Option<ResolvedTransition>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ResolvedEffect {
    pub kind: String,
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ResolvedTransition {
    pub kind: String,
    pub progress: f64, // 0..1
    pub params: serde_json::Value,
}

pub struct Evaluator;

impl Evaluator {
    #[tracing::instrument(skip(comp))]
    pub fn eval_frame(comp: &Composition, frame: FrameIndex) -> WavyteResult<EvaluatedGraph> {
        comp.validate()?;
        if frame.0 >= comp.duration.0 {
            return Err(WavyteError::evaluation("frame is out of bounds"));
        }

        let mut nodes_with_key: Vec<((i32, usize, u64, String), EvaluatedClipNode)> = Vec::new();

        for (track_index, track) in comp.tracks.iter().enumerate() {
            for clip in &track.clips {
                if !clip.range.contains(frame) {
                    continue;
                }

                let node = eval_clip(comp, clip, frame, track.z_base)?;
                let sort_key = (
                    node.z,
                    track_index,
                    clip.range.start.0,
                    node.clip_id.clone(),
                );
                nodes_with_key.push((sort_key, node));
            }
        }

        nodes_with_key.sort_by(|a, b| a.0.cmp(&b.0));
        let nodes = nodes_with_key.into_iter().map(|(_, n)| n).collect();

        Ok(EvaluatedGraph { frame, nodes })
    }
}

fn eval_clip(
    comp: &Composition,
    clip: &Clip,
    frame: FrameIndex,
    track_z_base: i32,
) -> WavyteResult<EvaluatedClipNode> {
    let clip_local = FrameIndex(frame.0 - clip.range.start.0);
    let seed = stable_hash64(comp.seed, &clip.id);
    let ctx = SampleCtx {
        frame,
        fps: comp.fps,
        clip_local,
        seed,
    };

    let opacity = clip.props.opacity.sample(ctx)?.clamp(0.0, 1.0);
    let transform = clip.props.transform.sample(ctx)?.to_affine();

    let effects = clip
        .effects
        .iter()
        .map(resolve_effect)
        .collect::<WavyteResult<Vec<_>>>()?;

    Ok(EvaluatedClipNode {
        clip_id: clip.id.clone(),
        asset: clip.asset.clone(),
        z: track_z_base + clip.z_offset,
        transform,
        opacity,
        blend: clip.props.blend,
        effects,
        transition_in: resolve_transition_in(clip, frame),
        transition_out: resolve_transition_out(clip, frame),
    })
}

fn resolve_effect(e: &EffectInstance) -> WavyteResult<ResolvedEffect> {
    if e.kind.trim().is_empty() {
        return Err(WavyteError::evaluation("effect kind must be non-empty"));
    }
    Ok(ResolvedEffect {
        kind: e.kind.clone(),
        params: e.params.clone(),
    })
}

fn resolve_transition_in(clip: &Clip, frame: FrameIndex) -> Option<ResolvedTransition> {
    let spec = clip.transition_in.as_ref()?;
    resolve_transition_window(
        spec,
        frame,
        clip.range,
        clip.range.start,
        TransitionEdge::In,
    )
}

fn resolve_transition_out(clip: &Clip, frame: FrameIndex) -> Option<ResolvedTransition> {
    let spec = clip.transition_out.as_ref()?;
    resolve_transition_window(spec, frame, clip.range, clip.range.end, TransitionEdge::Out)
}

#[derive(Clone, Copy, Debug)]
enum TransitionEdge {
    In,
    Out,
}

fn resolve_transition_window(
    spec: &TransitionSpec,
    frame: FrameIndex,
    clip_range: FrameRange,
    edge_frame: FrameIndex,
    edge: TransitionEdge,
) -> Option<ResolvedTransition> {
    if spec.duration_frames == 0 {
        return None;
    }

    let clip_len = clip_range.len_frames();
    if clip_len == 0 {
        return None;
    }
    let dur = spec.duration_frames.min(clip_len);

    let (window_start, window_end_excl) = match edge {
        TransitionEdge::In => {
            let start = edge_frame.0;
            let end = start.saturating_add(dur);
            (FrameIndex(start), FrameIndex(end))
        }
        TransitionEdge::Out => {
            let end = edge_frame.0;
            let start = end.saturating_sub(dur);
            (FrameIndex(start), FrameIndex(end))
        }
    };

    if !(window_start.0 <= frame.0 && frame.0 < window_end_excl.0) {
        return None;
    }

    let denom = dur.saturating_sub(1);
    let t = if denom == 0 {
        1.0
    } else {
        let offset = frame.0 - window_start.0;
        (offset as f64) / (denom as f64)
    };
    let progress = spec.ease.apply(t).clamp(0.0, 1.0);

    Some(ResolvedTransition {
        kind: spec.kind.clone(),
        progress,
        params: spec.params.clone(),
    })
}

fn stable_hash64(seed: u64, s: &str) -> u64 {
    // FNV-1a 64, seeded.
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for &b in s.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        anim::Anim,
        anim_ease::Ease,
        core::{Canvas, Fps, Transform2D, Vec2},
        model::{Asset, ClipProps, TextAsset, Track},
    };
    use std::collections::BTreeMap;

    fn basic_comp(
        opacity: Anim<f64>,
        tr_in: Option<TransitionSpec>,
        tr_out: Option<TransitionSpec>,
    ) -> Composition {
        let mut assets = BTreeMap::new();
        assets.insert(
            "t0".to_string(),
            Asset::Text(TextAsset {
                text: "hello".to_string(),
                font_source: "assets/PlayfairDisplay.ttf".to_string(),
                size_px: 48.0,
                max_width_px: None,
                color_rgba8: [255, 255, 255, 255],
            }),
        );
        Composition {
            fps: Fps::new(30, 1).unwrap(),
            canvas: Canvas {
                width: 640,
                height: 360,
            },
            duration: FrameIndex(20),
            assets,
            tracks: vec![Track {
                name: "main".to_string(),
                z_base: 0,
                clips: vec![Clip {
                    id: "c0".to_string(),
                    asset: "t0".to_string(),
                    range: FrameRange::new(FrameIndex(5), FrameIndex(15)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(Transform2D {
                            translate: Vec2::new(1.0, 2.0),
                            ..Transform2D::default()
                        }),
                        opacity,
                        blend: BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![],
                    transition_in: tr_in,
                    transition_out: tr_out,
                }],
            }],
            seed: 1,
        }
    }

    #[test]
    fn visibility_respects_frame_range() {
        let comp = basic_comp(Anim::constant(1.0), None, None);
        assert_eq!(
            Evaluator::eval_frame(&comp, FrameIndex(4))
                .unwrap()
                .nodes
                .len(),
            0
        );
        assert_eq!(
            Evaluator::eval_frame(&comp, FrameIndex(5))
                .unwrap()
                .nodes
                .len(),
            1
        );
        assert_eq!(
            Evaluator::eval_frame(&comp, FrameIndex(14))
                .unwrap()
                .nodes
                .len(),
            1
        );
        assert_eq!(
            Evaluator::eval_frame(&comp, FrameIndex(15))
                .unwrap()
                .nodes
                .len(),
            0
        );
    }

    #[test]
    fn opacity_is_clamped() {
        let opacity = Anim::constant(2.0);
        let comp = basic_comp(opacity, None, None);
        let g = Evaluator::eval_frame(&comp, FrameIndex(5)).unwrap();
        assert_eq!(g.nodes[0].opacity, 1.0);
    }

    #[test]
    fn transition_progress_boundaries() {
        let tr = TransitionSpec {
            kind: "crossfade".to_string(),
            duration_frames: 3,
            ease: Ease::Linear,
            params: serde_json::Value::Null,
        };
        let comp = basic_comp(Anim::constant(1.0), Some(tr.clone()), Some(tr));

        // In transition at clip start frame.
        let g0 = Evaluator::eval_frame(&comp, FrameIndex(5)).unwrap();
        assert_eq!(g0.nodes[0].transition_in.as_ref().unwrap().progress, 0.0);

        // Last in-transition frame hits progress 1.0 (dur=3 => denom=2).
        let g_last_in = Evaluator::eval_frame(&comp, FrameIndex(7)).unwrap();
        assert_eq!(
            g_last_in.nodes[0].transition_in.as_ref().unwrap().progress,
            1.0
        );

        // Out transition starts at end-dur.
        let g_out0 = Evaluator::eval_frame(&comp, FrameIndex(12)).unwrap();
        assert_eq!(
            g_out0.nodes[0].transition_out.as_ref().unwrap().progress,
            0.0
        );

        let g_out_last = Evaluator::eval_frame(&comp, FrameIndex(14)).unwrap();
        assert_eq!(
            g_out_last.nodes[0]
                .transition_out
                .as_ref()
                .unwrap()
                .progress,
            1.0
        );
    }
}
