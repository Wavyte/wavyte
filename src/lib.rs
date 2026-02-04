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
mod error;
mod eval;
mod model;
mod pipeline;
mod render;

#[cfg(feature = "cpu")]
mod render_cpu;
#[cfg(feature = "gpu")]
mod render_vello;

pub use anim::{Anim, InterpMode, Keyframe, Keyframes, LoopMode, SampleCtx};
pub use anim_ease::Ease;
pub use anim_ops::{delay, loop_, mix, reverse, sequence, speed, stagger};
pub use assets::{
    AssetCache, AssetId, AssetKey, FsAssetCache, PreparedAsset, PreparedImage, PreparedSvg,
    PreparedText, TextBrushRgba8, TextLayoutEngine,
};
pub use assets_decode::{decode_image, parse_svg};
pub use compile::{DrawOp, Pass, RenderPlan, ScenePass, compile_frame};
pub use core::{
    Affine, BezPath, Canvas, Fps, FrameIndex, FrameRange, Point, Rect, Rgba8Premul, Transform2D,
    Vec2,
};
pub use dsl::{ClipBuilder, CompositionBuilder, TrackBuilder};
pub use error::{WavyteError, WavyteResult};
pub use eval::{EvaluatedClipNode, EvaluatedGraph, Evaluator, ResolvedEffect, ResolvedTransition};
pub use model::{
    Asset, AudioAsset, BlendMode, Clip, ClipProps, Composition, EffectInstance, ImageAsset,
    PathAsset, SvgAsset, TextAsset, Track, TransitionSpec, VideoAsset,
};
pub use pipeline::render_frame;
pub use render::{BackendKind, FrameRGBA, RenderBackend, RenderSettings, create_backend};
