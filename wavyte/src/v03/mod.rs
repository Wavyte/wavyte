//! Wavyte v0.3 engine internals.
//!
//! The v0.3 public entrypoints are centered around:
//! - [`crate::v03::scene::composition::Composition`] (serde boundary + schema validation)
//! - [`crate::v03::session::render_session::RenderSession`] (session-oriented rendering)

#![allow(dead_code)]

pub(crate) mod animation;
pub(crate) mod assets;
pub(crate) mod audio;
pub(crate) mod compile;
pub(crate) mod effects;
pub mod encode;
pub(crate) mod eval;
pub(crate) mod expression;
pub(crate) mod foundation;
pub(crate) mod layout;
pub(crate) mod normalize;
pub mod render;
pub mod scene;
pub(crate) mod schema;
pub mod session;

// Public v0.3 entrypoints.
pub use crate::v03::encode::ffmpeg::{FfmpegSink, FfmpegSinkOpts};
pub use crate::v03::encode::sink::{AudioInputConfig, FrameSink, InMemorySink, SinkConfig};
pub use crate::v03::render::backend::FrameRGBA;
pub use crate::v03::render::cpu::CpuBackendOpts;
pub use crate::v03::scene::composition::Composition;
pub use crate::v03::session::render_session::{RenderSession, RenderSessionOpts, RenderStats};
