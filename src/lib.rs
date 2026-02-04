#![forbid(unsafe_code)]

pub mod core;
pub mod error;

pub use core::{Canvas, Fps, FrameIndex, FrameRange, Rgba8Premul, Transform2D, Vec2};
pub use error::{WavyteError, WavyteResult};
