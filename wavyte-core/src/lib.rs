//! Wavyte is a programmatic video composition and rendering engine.
//!
//! Wavyte v0.2.1 focuses on a stable and performant CPU-first pipeline that turns a
//! timeline (`Composition`) into pixels (`FrameRGBA`) via a backend-agnostic render IR (`RenderPlan`).
//!
//! # Pipeline overview
//!
//! 1. **Evaluate**: `Composition + FrameIndex -> EvaluatedGraph` (what is visible, in what order)
//! 2. **Compile**: `EvaluatedGraph -> RenderPlan` (backend-agnostic passes over explicit surfaces)
//! 3. **Render**: `RenderPlan -> FrameRGBA` (CPU backend)
//! 4. **Encode** (optional): stream frames to the system `ffmpeg` binary for MP4 output
//!
//! The key design constraints in v0.2.1:
//!
//! - **No unsafe**: `unsafe` is forbidden in this crate.
//! - **Deterministic-by-default**: evaluation/compilation are pure and stable for a given input.
//! - **No IO in renderers**: external IO is front-loaded in [`PreparedAssetStore`].
//! - **Premultiplied RGBA8** end-to-end: renderers output premultiplied pixels.
//!
//! # Getting started
//!
//! - For end-user usage, see the repository README.
//! - For a detailed, standalone walkthrough of the API and architecture, see [`crate::guide`].
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![allow(missing_docs_in_private_items)]

mod animation;
mod assets;
mod audio;
mod compile;
mod composition;
mod effects;
mod encode;
mod eval;
mod foundation;
mod layout;
mod render;

/// High-level, standalone documentation for Wavyte’s concepts and architecture.
pub mod guide;
/// Shared transform helpers (linear, affine, non-linear utilities).
pub mod transform;

pub use animation::anim::{Anim, InterpMode, Keyframe, Keyframes, LoopMode, SampleCtx};
pub use animation::ease::Ease;
pub use animation::ops::{delay, loop_, mix, reverse, sequence, speed, stagger};
pub use assets::decode::{decode_image, parse_svg};
pub use assets::media::{
    AudioPcm, MIX_SAMPLE_RATE, VideoSourceInfo, audio_source_time_sec, decode_audio_f32_stereo,
    decode_video_frame_rgba8, probe_video, video_source_time_sec,
};
pub use assets::store::{
    AssetId, AssetKey, PreparedAsset, PreparedAssetStore, PreparedAudio, PreparedImage,
    PreparedPath, PreparedSvg, PreparedText, PreparedVideo, TextBrushRgba8, TextLayoutEngine,
    normalize_rel_path,
};
pub use audio::mix::{
    AudioManifest, AudioSegment, build_audio_manifest, frame_to_sample, mix_manifest,
    write_mix_to_f32le_file,
};
pub use compile::fingerprint::{FrameFingerprint, fingerprint_eval};
pub use compile::plan::{
    CompositeOp, CompositePass, DrawOp, OffscreenPass, Pass, PixelFormat, RenderPlan, ScenePass,
    SurfaceDesc, SurfaceId, compile_frame,
};
pub use composition::dsl::{
    ClipBuilder, CompositionBuilder, TrackBuilder, audio_asset, video_asset,
};
pub use composition::model::{
    Asset, AudioAsset, BlendMode, Clip, ClipProps, Composition, Edges, EffectInstance, ImageAsset,
    LayoutAlignX, LayoutAlignY, LayoutMode, PathAsset, SvgAsset, TextAsset, Track, TransitionSpec,
    VideoAsset,
};
pub use effects::fx::{Effect, FxPipeline, InlineFx, PassFx, normalize_effects, parse_effect};
pub use effects::transitions::{TransitionKind, WipeDir, parse_transition};
pub use eval::evaluator::{
    EvaluatedClipNode, EvaluatedGraph, Evaluator, ResolvedEffect, ResolvedTransition,
};
pub use foundation::core::{
    Affine, BezPath, Canvas, Fps, FrameIndex, FrameRange, Point, Rect, Rgba8Premul, Transform2D,
    Vec2,
};
pub use foundation::error::{WavyteError, WavyteResult};
pub use layout::solver::{LayoutOffsets, resolve_layout_offsets};
pub use render::backend::{BackendKind, FrameRGBA, RenderBackend, RenderSettings, create_backend};
pub use render::cpu::CpuBackend;
pub use render::passes::{PassBackend, execute_plan};
pub use render::pipeline::{
    RenderStats, RenderThreading, RenderToMp4Opts, render_frame, render_frames,
    render_frames_with_stats, render_to_mp4, render_to_mp4_with_stats,
};

pub use encode::ffmpeg::{
    AudioInputConfig, EncodeConfig, FfmpegEncoder, default_mp4_config, ensure_parent_dir,
    is_ffmpeg_on_path,
};
