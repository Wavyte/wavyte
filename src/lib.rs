#![forbid(unsafe_code)]

mod anim;
mod anim_ease;
mod anim_ops;
mod anim_proc;
mod assets;
mod assets_decode;
mod core;
mod dsl;
mod error;
mod eval;
mod model;

pub use anim::{Anim, InterpMode, Keyframe, Keyframes, LoopMode, SampleCtx};
pub use anim_ease::Ease;
pub use anim_ops::{delay, loop_, mix, reverse, sequence, speed, stagger};
pub use assets::{PreparedImage, PreparedSvg};
pub use assets_decode::{decode_image, parse_svg};
pub use core::{Canvas, Fps, FrameIndex, FrameRange, Rgba8Premul, Transform2D, Vec2};
pub use dsl::{ClipBuilder, CompositionBuilder, TrackBuilder};
pub use error::{WavyteError, WavyteResult};
pub use eval::{EvaluatedClipNode, EvaluatedGraph, Evaluator, ResolvedEffect, ResolvedTransition};
pub use model::{
    Asset, AudioAsset, BlendMode, Clip, ClipProps, Composition, EffectInstance, ImageAsset,
    PathAsset, SvgAsset, TextAsset, Track, TransitionSpec, VideoAsset,
};
