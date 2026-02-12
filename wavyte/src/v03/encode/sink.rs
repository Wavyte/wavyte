use crate::foundation::core::{Fps, FrameIndex};
use crate::foundation::error::WavyteResult;
use crate::v03::render::backend::FrameRGBA;
use std::path::PathBuf;

/// Configuration provided to a [`FrameSink`] at the start of a range render.
#[derive(Debug, Clone)]
pub(crate) struct SinkConfig {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: Fps,
    /// Optional external raw PCM audio file input.
    pub(crate) audio: Option<AudioInputConfig>,
}

#[derive(Debug, Clone)]
pub(crate) struct AudioInputConfig {
    pub(crate) path: PathBuf,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
}

/// Sink contract for consuming rendered frames in timeline order.
///
/// Ordering contract: `push_frame` is called in strictly increasing `FrameIndex` order within the
/// requested render range.
pub(crate) trait FrameSink: Send {
    fn begin(&mut self, cfg: SinkConfig) -> WavyteResult<()>;
    fn push_frame(&mut self, idx: FrameIndex, frame: &FrameRGBA) -> WavyteResult<()>;
    fn end(&mut self) -> WavyteResult<()>;
}

/// In-memory sink for tests and debugging.
#[derive(Debug, Default)]
pub(crate) struct InMemorySink {
    cfg: Option<SinkConfig>,
    /// Frames in timeline order.
    pub(crate) frames: Vec<(FrameIndex, FrameRGBA)>,
}

impl InMemorySink {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn config(&self) -> Option<SinkConfig> {
        self.cfg.clone()
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
