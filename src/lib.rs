#![forbid(unsafe_code)]

pub mod anim;
pub mod anim_ease;
pub mod anim_proc;
pub mod core;
pub mod error;

pub use anim::Anim;
pub use anim_ease::Ease;
pub use core::{Canvas, Fps, FrameIndex, FrameRange, Rgba8Premul, Transform2D, Vec2};
pub use error::{WavyteError, WavyteResult};
