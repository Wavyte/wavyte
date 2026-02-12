//! Wavyte is a programmatic video composition and rendering engine.
//!
//! v0.3 is a full internal rewrite focused on production-grade determinism and hot-loop
//! performance. The public API is session-oriented:
//!
//! - Load and validate a [`Composition`]
//! - Create a [`RenderSession`]
//! - Render single frames or stream a range into a [`FrameSink`]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
// v0.3 keeps some scaffolding and reserved IR variants that are intentionally not exercised by the
// minimal public surface yet. We keep `clippy -D warnings` clean by allowing `dead_code` at the
// crate level (similar to how v0.3 previously scoped this to the `v03` module).
#![allow(dead_code)]

mod assets;
mod foundation;

pub(crate) mod animation;
pub(crate) mod audio;
pub(crate) mod compile;
pub(crate) mod effects;
/// v0.3 encoding sinks.
pub mod encode;
pub(crate) mod eval;
pub(crate) mod expression;
pub(crate) mod layout;
pub(crate) mod normalize;
/// v0.3 rendering backend(s).
pub mod render;
/// v0.3 boundary scene model.
pub mod scene;
pub(crate) mod schema;
/// v0.3 session-oriented rendering API.
pub mod session;

pub use crate::foundation::core::{
    Affine, BezPath, Canvas, Fps, FrameIndex, FrameRange, Point, Rect, Rgba8Premul, Vec2,
};
pub use crate::foundation::error::{WavyteError, WavyteResult};

pub use crate::encode::ffmpeg::{FfmpegSink, FfmpegSinkOpts};
pub use crate::encode::sink::{AudioInputConfig, FrameSink, InMemorySink, SinkConfig};
pub use crate::render::backend::FrameRGBA;
pub use crate::render::cpu::CpuBackendOpts;
pub use crate::scene::composition::Composition;
pub use crate::session::render_session::{RenderSession, RenderSessionOpts, RenderStats};
