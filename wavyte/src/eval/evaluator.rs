use crate::{
    animation::anim::SampleCtx,
    composition::model::{Asset, BlendMode, Clip, Composition, EffectInstance, TransitionSpec},
    foundation::core::{FrameIndex, FrameRange},
    foundation::error::{WavyteError, WavyteResult},
};

#[derive(Clone, Debug, serde::Serialize)]
/// Fully evaluated frame graph before compilation.
pub struct EvaluatedGraph {
    /// Evaluated frame index.
    pub frame: FrameIndex,
    /// Visible clip nodes in painter's order.
    pub nodes: Vec<EvaluatedClipNode>,
}

#[derive(Clone, Debug, serde::Serialize)]
/// Evaluated clip node consumed by the compiler.
pub struct EvaluatedClipNode {
    /// Clip identifier.
    pub clip_id: String,
    /// Referenced composition asset key.
    pub asset: String,
    /// Absolute z-order after track and clip offsets.
    pub z: i32,
    /// Fully resolved transform matrix.
    pub transform: kurbo::Affine,
    /// Final intrinsic opacity in `[0, 1]`.
    pub opacity: f64,
    /// Blend mode for compositing.
    pub blend: BlendMode,
    /// Source media time (for video clips), if applicable.
    pub source_time_s: Option<f64>,
    /// Effects copied from clip and validated for compile.
    pub effects: Vec<ResolvedEffect>,
    /// Optional resolved transition-in state.
    pub transition_in: Option<ResolvedTransition>,
    /// Optional resolved transition-out state.
    pub transition_out: Option<ResolvedTransition>,
}

#[derive(Clone, Debug, serde::Serialize)]
/// Effect instance resolved for a specific evaluated node.
pub struct ResolvedEffect {
    /// Canonical effect kind identifier.
    pub kind: String,
    /// Raw effect parameters.
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, serde::Serialize)]
/// Transition state resolved for a specific frame.
pub struct ResolvedTransition {
    /// Canonical transition kind identifier.
    pub kind: String,
    /// Transition progress in `[0, 1]`.
    pub progress: f64, // 0..1
    /// Raw transition parameters.
    pub params: serde_json::Value,
}

/// Stateless evaluator from composition timeline to frame graph.
pub struct Evaluator;

impl Evaluator {
    #[tracing::instrument(skip(comp))]
    /// Evaluate one frame using default (zero) layout offsets.
    pub fn eval_frame(comp: &Composition, frame: FrameIndex) -> WavyteResult<EvaluatedGraph> {
        Self::eval_frame_with_layout_impl(comp, frame, &crate::LayoutOffsets::default(), true)
    }

    #[tracing::instrument(skip(comp, layout))]
    /// Evaluate one frame with precomputed layout offsets.
    pub fn eval_frame_with_layout(
        comp: &Composition,
        frame: FrameIndex,
        layout: &crate::LayoutOffsets,
    ) -> WavyteResult<EvaluatedGraph> {
        Self::eval_frame_with_layout_impl(comp, frame, layout, true)
    }

    pub(crate) fn eval_frame_with_layout_unchecked(
        comp: &Composition,
        frame: FrameIndex,
        layout: &crate::LayoutOffsets,
    ) -> WavyteResult<EvaluatedGraph> {
        Self::eval_frame_with_layout_impl(comp, frame, layout, false)
    }

    fn eval_frame_with_layout_impl(
        comp: &Composition,
        frame: FrameIndex,
        layout: &crate::LayoutOffsets,
        validate_comp: bool,
    ) -> WavyteResult<EvaluatedGraph> {
        if validate_comp {
            comp.validate()?;
        }
        if frame.0 >= comp.duration.0 {
            return Err(WavyteError::evaluation("frame is out of bounds"));
        }

        let mut nodes_with_key: Vec<((i32, usize, u64, String), EvaluatedClipNode)> = Vec::new();

        for (track_index, track) in comp.tracks.iter().enumerate() {
            for (clip_index, clip) in track.clips.iter().enumerate() {
                if !clip.range.contains(frame) {
                    continue;
                }

                let node = eval_clip(
                    comp,
                    clip,
                    frame,
                    track.z_base,
                    layout.offset_for(track_index, clip_index),
                )?;
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
    layout_offset: crate::foundation::core::Vec2,
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
    let transform = kurbo::Affine::translate((layout_offset.x, layout_offset.y))
        * clip.props.transform.sample(ctx)?.to_affine();
    let source_time_s = match comp.assets.get(&clip.asset) {
        Some(Asset::Video(video)) => Some(crate::assets::media::video_source_time_sec(
            video,
            clip_local.0,
            comp.fps,
        )),
        _ => None,
    };

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
        source_time_s,
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
#[path = "../../tests/unit/eval/evaluator.rs"]
mod tests;
