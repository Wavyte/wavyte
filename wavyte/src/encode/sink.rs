use crate::foundation::core::{Fps, FrameIndex};
use crate::foundation::error::WavyteResult;
use crate::render::backend::FrameRGBA;
use std::path::PathBuf;

/// Configuration provided to a [`FrameSink`] at the start of a range render.
#[derive(Debug, Clone)]
pub struct SinkConfig {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Output frames-per-second.
    pub fps: Fps,
    /// Optional external raw PCM audio file input.
    pub audio: Option<AudioInputConfig>,
}

/// Raw PCM audio input configuration for sinks that support audio encoding.
#[derive(Debug, Clone)]
pub struct AudioInputConfig {
    /// Path to interleaved `f32le` PCM data.
    pub path: PathBuf,
    /// Sample rate in Hz (v0.3 default is 48_000).
    pub sample_rate: u32,
    /// Channel count (v0.3 default is 2).
    pub channels: u16,
}

/// Sink contract for consuming rendered frames in timeline order.
///
/// Ordering contract: `push_frame` is called in strictly increasing `FrameIndex` order within the
/// requested render range.
pub trait FrameSink: Send {
    /// Called once before any frames are pushed.
    fn begin(&mut self, cfg: SinkConfig) -> WavyteResult<()>;
    /// Push one frame in strictly increasing timeline order.
    fn push_frame(&mut self, idx: FrameIndex, frame: &FrameRGBA) -> WavyteResult<()>;
    /// Called once after the last frame is pushed.
    fn end(&mut self) -> WavyteResult<()>;
}

/// In-memory sink for tests and debugging.
#[derive(Debug, Default)]
pub struct InMemorySink {
    cfg: Option<SinkConfig>,
    /// Frames in timeline order.
    pub(crate) frames: Vec<(FrameIndex, FrameRGBA)>,
}

impl InMemorySink {
    /// Create a new in-memory sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the sink configuration captured in `begin`, if any.
    pub fn config(&self) -> Option<SinkConfig> {
        self.cfg.clone()
    }

    /// Borrow the captured frames.
    pub fn frames(&self) -> &[(FrameIndex, FrameRGBA)] {
        &self.frames
    }
}

impl FrameSink for InMemorySink {
    fn begin(&mut self, cfg: SinkConfig) -> WavyteResult<()> {
        self.cfg = Some(cfg);
        self.frames.clear();
        Ok(())
    }

    fn push_frame(&mut self, idx: FrameIndex, frame: &FrameRGBA) -> WavyteResult<()> {
        self.frames.push((idx, frame.clone()));
        Ok(())
    }

    fn end(&mut self) -> WavyteResult<()> {
        Ok(())
    }
}
