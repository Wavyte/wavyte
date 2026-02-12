//! Wavyte is a programmatic video composition and rendering engine.
//!
//! v0.3 is a full internal rewrite focused on production-grade determinism and hot-loop
//! performance. The public API is session-oriented:
//!
//! - Load and validate a [`v03::Composition`]
//! - Create a [`v03::RenderSession`]
//! - Render single frames or stream a range into a [`v03::FrameSink`]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod assets;
mod foundation;

/// v0.3 engine (core implementation and public entrypoints).
pub mod v03;

pub use crate::foundation::core::{
    Affine, BezPath, Canvas, Fps, FrameIndex, FrameRange, Point, Rect, Rgba8Premul, Vec2,
};
pub use crate::foundation::error::{WavyteError, WavyteResult};

pub use crate::v03::{
    AudioInputConfig, Composition, CpuBackendOpts, FfmpegSink, FfmpegSinkOpts, FrameRGBA,
    FrameSink, InMemorySink, RenderSession, RenderSessionOpts, RenderStats, SinkConfig,
};
