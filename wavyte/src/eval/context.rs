#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NodeTimeCtx {
    /// Node-local frame before clamping. May be negative/outside duration.
    pub(crate) local_frame_i64: i64,
    pub(crate) duration_frames: u32,
}

impl NodeTimeCtx {
    pub(crate) fn is_in_range(self) -> bool {
        self.local_frame_i64 >= 0 && self.local_frame_i64 < i64::from(self.duration_frames)
    }

    pub(crate) fn duration_frames_u64(self) -> u64 {
        u64::from(self.duration_frames)
    }

    /// `time.frame` for expressions: clamped to `[0, time.duration]` (inclusive upper bound).
    pub(crate) fn time_frame_u64(self) -> u64 {
        let dur = self.duration_frames_u64();
        self.local_frame_i64.clamp(0, dur as i64) as u64
    }

    pub(crate) fn time_frame_f64(self) -> f64 {
        self.time_frame_u64() as f64
    }

    /// Sampling frame for `Anim<T>`: clamped to `[0, duration_frames.saturating_sub(1)]`.
    pub(crate) fn sample_frame_u64(self) -> u64 {
        let dur = self.duration_frames_u64();
        if dur == 0 {
            return 0;
        }
        let max = dur - 1;
        self.local_frame_i64.clamp(0, max as i64) as u64
    }

    pub(crate) fn sample_frame_f64(self) -> f64 {
        self.sample_frame_u64() as f64
    }

    pub(crate) fn duration_f64(self) -> f64 {
        f64::from(self.duration_frames)
    }

    pub(crate) fn progress_f64(self) -> f64 {
        let dur = self.duration_f64();
        if dur <= 0.0 {
            0.0
        } else {
            (self.time_frame_f64() / dur).clamp(0.0, 1.0)
        }
    }
}
