//! Wavyte is a programmatic video composition and rendering engine.
//!
//! Wavyte v0.1.0 is deliberately scoped: it focuses on a stable and testable pipeline that turns a
//! timeline (`Composition`) into pixels (`FrameRGBA`) via a backend-agnostic render IR (`RenderPlan`).
//!
//! # Pipeline overview
//!
//! 1. **Evaluate**: `Composition + FrameIndex -> EvaluatedGraph` (what is visible, in what order)
//! 2. **Compile**: `EvaluatedGraph -> RenderPlan` (backend-agnostic passes over explicit surfaces)
//! 3. **Render**: `RenderPlan -> FrameRGBA` (CPU by default, optional GPU backend)
//! 4. **Encode** (optional): stream frames to the system `ffmpeg` binary for MP4 output
//!
//! The key design constraints in v0.1.0:
//!
//! - **No unsafe**: `unsafe` is forbidden in this crate.
//! - **Deterministic-by-default**: evaluation/compilation are pure and stable for a given input.
//! - **No IO in renderers**: external IO lives behind [`AssetCache`].
//! - **Premultiplied RGBA8** end-to-end: renderers output premultiplied pixels.
//!
//! # Getting started
//!
//! - For end-user usage, see the repository README.
//! - For a detailed, standalone walkthrough of the API and architecture, see [`crate::guide`].
#![forbid(unsafe_code)]

mod anim;
mod anim_ease;
mod anim_ops;
mod anim_proc;
mod assets;
mod assets_decode;
mod compile;
mod core;
mod dsl;
mod encode_ffmpeg;
mod error;
mod eval;
mod fx;
mod model;
mod pipeline;
mod render;
mod render_passes;
mod svg_raster;
mod transitions;

mod blur_cpu;
mod composite_cpu;
mod render_cpu;

/// High-level, standalone documentation for Wavyte’s concepts and architecture.
pub mod guide;

pub use anim::{Anim, InterpMode, Keyframe, Keyframes, LoopMode, SampleCtx};
pub use anim_ease::Ease;
pub use anim_ops::{delay, loop_, mix, reverse, sequence, speed, stagger};
pub use assets::{
    AssetCache, AssetId, AssetKey, FsAssetCache, PreparedAsset, PreparedImage, PreparedSvg,
    PreparedText, TextBrushRgba8, TextLayoutEngine,
};
pub use assets_decode::{decode_image, parse_svg};
pub use compile::{
    CompositeOp, CompositePass, DrawOp, OffscreenPass, Pass, PixelFormat, RenderPlan, ScenePass,
    SurfaceDesc, SurfaceId, compile_frame,
};
pub use core::{
    Affine, BezPath, Canvas, Fps, FrameIndex, FrameRange, Point, Rect, Rgba8Premul, Transform2D,
    Vec2,
};
pub use dsl::{ClipBuilder, CompositionBuilder, TrackBuilder};
pub use error::{WavyteError, WavyteResult};
pub use eval::{EvaluatedClipNode, EvaluatedGraph, Evaluator, ResolvedEffect, ResolvedTransition};
pub use fx::{Effect, FxPipeline, InlineFx, PassFx, normalize_effects, parse_effect};
pub use model::{
    Asset, AudioAsset, BlendMode, Clip, ClipProps, Composition, EffectInstance, ImageAsset,
    PathAsset, SvgAsset, TextAsset, Track, TransitionSpec, VideoAsset,
};
pub use pipeline::{RenderToMp4Opts, render_frame, render_frames, render_to_mp4};
pub use render::{BackendKind, FrameRGBA, RenderBackend, RenderSettings, create_backend};
pub use render_cpu::CpuBackend;
pub use render_passes::{PassBackend, execute_plan};
pub use transitions::{TransitionKind, WipeDir, parse_transition};

pub use encode_ffmpeg::{
    EncodeConfig, FfmpegEncoder, default_mp4_config, ensure_parent_dir, is_ffmpeg_on_path,
};
