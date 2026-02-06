//! Wavyte is a programmatic video composition and rendering engine.
//!
//! Wavyte v0.2.0 focuses on a stable and performant CPU-first pipeline that turns a
//! timeline (`Composition`) into pixels (`FrameRGBA`) via a backend-agnostic render IR (`RenderPlan`).
//!
//! # Pipeline overview
//!
//! 1. **Evaluate**: `Composition + FrameIndex -> EvaluatedGraph` (what is visible, in what order)
//! 2. **Compile**: `EvaluatedGraph -> RenderPlan` (backend-agnostic passes over explicit surfaces)
//! 3. **Render**: `RenderPlan -> FrameRGBA` (CPU backend)
//! 4. **Encode** (optional): stream frames to the system `ffmpeg` binary for MP4 output
//!
//! The key design constraints in v0.2.0:
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

#[path = "animation/anim.rs"]
mod anim;
#[path = "animation/ease.rs"]
mod anim_ease;
#[path = "animation/ops.rs"]
mod anim_ops;
#[path = "animation/proc.rs"]
mod anim_proc;
#[path = "assets/store.rs"]
mod asset_store;
#[path = "assets/decode.rs"]
mod assets_decode;
#[path = "audio/mix.rs"]
mod audio_mix;
#[path = "render/compile.rs"]
mod compile;
#[path = "foundation/core.rs"]
mod core;
#[path = "composition/dsl.rs"]
mod dsl;
#[path = "render/encode_ffmpeg.rs"]
mod encode_ffmpeg;
#[path = "foundation/error.rs"]
mod error;
#[path = "composition/eval.rs"]
mod eval;
#[path = "render/fingerprint.rs"]
mod fingerprint;
#[path = "render/fx.rs"]
mod fx;
#[path = "composition/layout.rs"]
mod layout;
#[path = "assets/media.rs"]
mod media;
#[path = "composition/model.rs"]
mod model;
#[path = "render/pipeline.rs"]
mod pipeline;
#[path = "render/backend.rs"]
mod render;
#[path = "render/passes.rs"]
mod render_passes;
#[path = "assets/svg_raster.rs"]
mod svg_raster;
#[path = "render/transitions.rs"]
mod transitions;

#[path = "render/blur.rs"]
mod blur_cpu;
#[path = "render/composite.rs"]
mod composite_cpu;
#[path = "render/cpu.rs"]
mod render_cpu;

/// High-level, standalone documentation for Wavyte’s concepts and architecture.
pub mod guide;

pub use anim::{Anim, InterpMode, Keyframe, Keyframes, LoopMode, SampleCtx};
pub use anim_ease::Ease;
pub use anim_ops::{delay, loop_, mix, reverse, sequence, speed, stagger};
pub use asset_store::{
    AssetId, AssetKey, PreparedAsset, PreparedAssetStore, PreparedAudio, PreparedImage,
    PreparedPath, PreparedSvg, PreparedText, PreparedVideo, TextBrushRgba8, TextLayoutEngine,
    normalize_rel_path,
};
pub use assets_decode::{decode_image, parse_svg};
pub use audio_mix::{
    AudioManifest, AudioSegment, build_audio_manifest, frame_to_sample, mix_manifest,
    write_mix_to_f32le_file,
};
pub use compile::{
    CompositeOp, CompositePass, DrawOp, OffscreenPass, Pass, PixelFormat, RenderPlan, ScenePass,
    SurfaceDesc, SurfaceId, compile_frame,
};
pub use core::{
    Affine, BezPath, Canvas, Fps, FrameIndex, FrameRange, Point, Rect, Rgba8Premul, Transform2D,
    Vec2,
};
pub use dsl::{ClipBuilder, CompositionBuilder, TrackBuilder, audio_asset, video_asset};
pub use error::{WavyteError, WavyteResult};
pub use eval::{EvaluatedClipNode, EvaluatedGraph, Evaluator, ResolvedEffect, ResolvedTransition};
pub use fingerprint::{FrameFingerprint, fingerprint_eval};
pub use fx::{Effect, FxPipeline, InlineFx, PassFx, normalize_effects, parse_effect};
pub use layout::{LayoutOffsets, resolve_layout_offsets};
pub use media::{
    AudioPcm, MIX_SAMPLE_RATE, VideoSourceInfo, audio_source_time_sec, decode_audio_f32_stereo,
    decode_video_frame_rgba8, probe_video, video_source_time_sec,
};
pub use model::{
    Asset, AudioAsset, BlendMode, Clip, ClipProps, Composition, Edges, EffectInstance, ImageAsset,
    LayoutAlignX, LayoutAlignY, LayoutMode, PathAsset, SvgAsset, TextAsset, Track, TransitionSpec,
    VideoAsset,
};
pub use pipeline::{
    RenderStats, RenderThreading, RenderToMp4Opts, render_frame, render_frames,
    render_frames_with_stats, render_to_mp4, render_to_mp4_with_stats,
};
pub use render::{BackendKind, FrameRGBA, RenderBackend, RenderSettings, create_backend};
pub use render_cpu::CpuBackend;
pub use render_passes::{PassBackend, execute_plan};
pub use transitions::{TransitionKind, WipeDir, parse_transition};

pub use encode_ffmpeg::{
    AudioInputConfig, EncodeConfig, FfmpegEncoder, default_mp4_config, ensure_parent_dir,
    is_ffmpeg_on_path,
};
