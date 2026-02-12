//! v0.3 encoding sinks.
//!
//! Sinks consume rendered frames in timeline order and are used by `RenderSession::render_range`.

/// `ffmpeg`-based sinks (MP4 output via system `ffmpeg`).
pub mod ffmpeg;
/// Generic frame sink trait and built-in sinks.
pub mod sink;
