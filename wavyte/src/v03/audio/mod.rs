//! v0.3 audio pipeline (manifest building + mixing).
//!
//! Audio mixing is performed outside the per-frame render hot loop and fed to sinks (ffmpeg) as a
//! separate input.

pub(crate) mod manifest;
pub(crate) mod mix;
