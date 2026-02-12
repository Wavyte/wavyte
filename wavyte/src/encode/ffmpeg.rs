use crate::encode::sink::{FrameSink, SinkConfig};
use crate::foundation::core::{Fps, FrameIndex};
use crate::foundation::error::{WavyteError, WavyteResult};
use crate::foundation::math::mul_div255_u16;
use crate::render::backend::FrameRGBA;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

/// Options for [`FfmpegSink`] MP4 output.
#[derive(Clone, Debug)]
pub struct FfmpegSinkOpts {
    /// Output MP4 file path.
    pub out_path: PathBuf,
    /// Overwrite output file if it already exists.
    pub overwrite: bool,
    /// Background color used to flatten alpha (RGBA8, straight alpha).
    pub bg_rgba: [u8; 4],
}

impl FfmpegSinkOpts {
    /// Create options for outputting an MP4 to `out_path`.
    pub fn new(out_path: impl Into<PathBuf>) -> Self {
        Self {
            out_path: out_path.into(),
            overwrite: true,
            bg_rgba: [0, 0, 0, 255],
        }
    }
}

/// v0.3 sink that spawns the system `ffmpeg` and streams raw frames to stdin.
///
/// Audio is optional and provided through `SinkConfig.audio`.
pub struct FfmpegSink {
    opts: FfmpegSinkOpts,

    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stderr_drain: Option<std::thread::JoinHandle<std::io::Result<Vec<u8>>>>,

    scratch: Vec<u8>,
    cfg: Option<SinkConfig>,
    last_idx: Option<FrameIndex>,
}

impl FfmpegSink {
    /// Create a new sink that streams into `ffmpeg`.
    pub fn new(opts: FfmpegSinkOpts) -> Self {
        Self {
            opts,
            child: None,
            stdin: None,
            stderr_drain: None,
            scratch: Vec::new(),
            cfg: None,
            last_idx: None,
        }
    }
}

impl FrameSink for FfmpegSink {
    fn begin(&mut self, cfg: SinkConfig) -> WavyteResult<()> {
        if cfg.fps.num == 0 || cfg.fps.den == 0 {
            return Err(WavyteError::validation("fps must be non-zero"));
        }
        if cfg.width == 0 || cfg.height == 0 {
            return Err(WavyteError::validation(
                "ffmpeg sink width/height must be non-zero",
            ));
        }
        if !cfg.width.is_multiple_of(2) || !cfg.height.is_multiple_of(2) {
            return Err(WavyteError::validation(
                "ffmpeg sink width/height must be even (required for yuv420p mp4 output)",
            ));
        }

        ensure_parent_dir(&self.opts.out_path)?;
        if !self.opts.overwrite && self.opts.out_path.exists() {
            return Err(WavyteError::validation(format!(
                "output file '{}' already exists",
                self.opts.out_path.display()
            )));
        }

        if !is_ffmpeg_on_path() {
            return Err(WavyteError::evaluation(
                "ffmpeg is required for MP4 encoding, but was not found on PATH",
            ));
        }

        let mut cmd = Command::new("ffmpeg");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        if self.opts.overwrite {
            cmd.arg("-y");
        } else {
            cmd.arg("-n");
        }

        // Input: raw premultiplied RGBA8 frames. `ffmpeg` does not understand premul, so we
        // flatten alpha before writing to stdin (push_frame).
        cmd.args([
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{}x{}", cfg.width, cfg.height),
        ]);
        push_input_fps(&mut cmd, cfg.fps);
        cmd.args(["-i", "pipe:0"]);

        if let Some(audio) = cfg.audio.as_ref() {
            if audio.sample_rate == 0 {
                return Err(WavyteError::validation(
                    "audio sample_rate must be non-zero when audio is enabled",
                ));
            }
            if audio.channels == 0 {
                return Err(WavyteError::validation(
                    "audio channels must be non-zero when audio is enabled",
                ));
            }
            cmd.args([
                "-f",
                "f32le",
                "-ar",
                &audio.sample_rate.to_string(),
                "-ac",
                &audio.channels.to_string(),
                "-i",
            ])
            .arg(&audio.path)
            .args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
                "-movflags",
                "+faststart",
            ]);
        } else {
            // Output: h264 + yuv420p for broad compatibility.
            cmd.args([
                "-an",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ]);
        }
        cmd.arg(&self.opts.out_path);

        let mut child = cmd.spawn().map_err(|e| {
            WavyteError::evaluation(format!(
                "failed to spawn ffmpeg (is it installed and on PATH?): {e}"
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| WavyteError::evaluation("failed to open ffmpeg stdin (unexpected)"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| WavyteError::evaluation("failed to open ffmpeg stderr (unexpected)"))?;
        let stderr_drain = std::thread::spawn(move || {
            let mut stderr_bytes = Vec::new();
            stderr.read_to_end(&mut stderr_bytes)?;
            Ok(stderr_bytes)
        });

        self.scratch = vec![0u8; (cfg.width * cfg.height * 4) as usize];
        self.child = Some(child);
        self.stdin = Some(stdin);
        self.stderr_drain = Some(stderr_drain);
        self.cfg = Some(cfg);
        self.last_idx = None;
        Ok(())
    }

    fn push_frame(&mut self, idx: FrameIndex, frame: &FrameRGBA) -> WavyteResult<()> {
        let cfg = self
            .cfg
            .as_ref()
            .ok_or_else(|| WavyteError::evaluation("ffmpeg sink not started"))?;
        if let Some(last) = self.last_idx
            && idx.0 <= last.0
        {
            return Err(WavyteError::evaluation(
                "ffmpeg sink received out-of-order frame index",
            ));
        }
        self.last_idx = Some(idx);

        if frame.width != cfg.width || frame.height != cfg.height {
            return Err(WavyteError::validation(format!(
                "frame size mismatch: got {}x{}, expected {}x{}",
                frame.width, frame.height, cfg.width, cfg.height
            )));
        }
        if frame.data.len() != self.scratch.len() {
            return Err(WavyteError::validation(
                "frame.data size mismatch with width*height*4",
            ));
        }

        // Flatten premultiplied RGBA8 over the configured background.
        flatten_premul_over_bg_to_opaque_rgba8(&mut self.scratch, &frame.data, self.opts.bg_rgba)?;

        let Some(stdin) = self.stdin.as_mut() else {
            return Err(WavyteError::evaluation("ffmpeg sink is already finalized"));
        };

        use std::io::Write as _;
        stdin.write_all(&self.scratch).map_err(|e| {
            WavyteError::evaluation(format!("failed to write frame to ffmpeg stdin: {e}"))
        })?;
        Ok(())
    }

    fn end(&mut self) -> WavyteResult<()> {
        drop(self.stdin.take());
        let mut child = self
            .child
            .take()
            .ok_or_else(|| WavyteError::evaluation("ffmpeg sink not started"))?;

        let status = child.wait().map_err(|e| {
            WavyteError::evaluation(format!("failed to wait for ffmpeg to finish: {e}"))
        })?;
        let stderr_bytes = match self.stderr_drain.take() {
            Some(handle) => handle
                .join()
                .map_err(|_| WavyteError::evaluation("ffmpeg stderr drain thread panicked"))?
                .map_err(|e| WavyteError::evaluation(format!("ffmpeg stderr read failed: {e}")))?,
            None => Vec::new(),
        };

        if !status.success() {
            let stderr = String::from_utf8_lossy(&stderr_bytes);
            return Err(WavyteError::evaluation(format!(
                "ffmpeg exited with status {}: {}",
                status,
                stderr.trim()
            )));
        }

        self.cfg = None;
        Ok(())
    }
}

fn push_input_fps(cmd: &mut Command, fps: Fps) {
    // For rawvideo input, use `-r` before `-i` to specify the input framerate.
    //
    // Accept rational FPS as `num/den`.
    cmd.args(["-r", &format!("{}/{}", fps.num, fps.den)]);
}

fn flatten_premul_over_bg_to_opaque_rgba8(
    dst: &mut [u8],
    src_premul: &[u8],
    bg_rgba: [u8; 4],
) -> WavyteResult<()> {
    if dst.len() != src_premul.len() || !dst.len().is_multiple_of(4) {
        return Err(WavyteError::validation(
            "flatten_premul_over_bg_to_opaque_rgba8 expects equal-length rgba8 buffers",
        ));
    }

    let bg_r = bg_rgba[0] as u16;
    let bg_g = bg_rgba[1] as u16;
    let bg_b = bg_rgba[2] as u16;

    for (d, s) in dst.chunks_exact_mut(4).zip(src_premul.chunks_exact(4)) {
        let a = s[3] as u16;
        if a == 255 {
            d.copy_from_slice(s);
            d[3] = 255;
            continue;
        }

        let inv = 255u16 - a;
        let r = s[0] as u16 + mul_div255(bg_r, inv);
        let g = s[1] as u16 + mul_div255(bg_g, inv);
        let b = s[2] as u16 + mul_div255(bg_b, inv);

        d[0] = r.min(255) as u8;
        d[1] = g.min(255) as u8;
        d[2] = b.min(255) as u8;
        d[3] = 255;
    }

    Ok(())
}

fn mul_div255(x: u16, y: u16) -> u16 {
    mul_div255_u16(x, y)
}

/// Ensure the parent directory of `path` exists.
pub fn ensure_parent_dir(path: &Path) -> WavyteResult<()> {
    if let Some(parent) = path.parent() {
        use anyhow::Context as _;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory '{}'", parent.display()))?;
    }
    Ok(())
}

/// Return `true` when `ffmpeg` can be invoked from `PATH`.
pub fn is_ffmpeg_on_path() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_premul_alpha_0_returns_bg() {
        let src = vec![0u8, 0, 0, 0];
        let mut dst = vec![0u8; 4];
        flatten_premul_over_bg_to_opaque_rgba8(&mut dst, &src, [10, 20, 30, 255]).unwrap();
        assert_eq!(dst, vec![10, 20, 30, 255]);
    }

    #[test]
    fn flatten_premul_alpha_255_is_identity() {
        let src = vec![1u8, 2, 3, 255];
        let mut dst = vec![0u8; 4];
        flatten_premul_over_bg_to_opaque_rgba8(&mut dst, &src, [10, 20, 30, 255]).unwrap();
        assert_eq!(dst, src);
    }
}
