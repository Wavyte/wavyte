use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
};

use crate::{
    foundation::error::{WavyteError, WavyteResult},
    foundation::math::mul_div255_u16,
    render::FrameRGBA,
};

/// Configuration for MP4 encoding via the system `ffmpeg` binary.
#[derive(Clone, Debug)]
pub struct EncodeConfig {
    /// Output video width in pixels.
    pub width: u32,
    /// Output video height in pixels.
    pub height: u32,
    /// Output frame rate (integer FPS).
    pub fps: u32,
    /// Output MP4 path.
    pub out_path: PathBuf,
    /// Overwrite existing output file when `true`.
    pub overwrite: bool,
    /// Optional external audio input stream configuration.
    pub audio: Option<AudioInputConfig>,
}

#[derive(Clone, Debug)]
/// External raw PCM audio input fed into ffmpeg.
pub struct AudioInputConfig {
    /// Path to raw interleaved `f32le` audio file.
    pub path: PathBuf,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
}

impl EncodeConfig {
    /// Validate invariants required by the current MP4 encoder configuration.
    pub fn validate(&self) -> WavyteResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(WavyteError::validation(
                "encode width/height must be non-zero",
            ));
        }
        if self.fps == 0 {
            return Err(WavyteError::validation("encode fps must be non-zero"));
        }
        if !self.width.is_multiple_of(2) || !self.height.is_multiple_of(2) {
            // With the default settings we target yuv420p output for maximum compatibility.
            return Err(WavyteError::validation(
                "encode width/height must be even (required for yuv420p mp4 output)",
            ));
        }
        if let Some(audio) = &self.audio {
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
        }
        Ok(())
    }

    /// Return a copy of this config with a new output path.
    pub fn with_out_path(mut self, out_path: impl Into<PathBuf>) -> Self {
        self.out_path = out_path.into();
        self
    }
}

/// Create the default MP4 encoding config used by Wavyte’s pipeline APIs.
pub fn default_mp4_config(
    out_path: impl Into<PathBuf>,
    width: u32,
    height: u32,
    fps: u32,
) -> EncodeConfig {
    EncodeConfig {
        width,
        height,
        fps,
        out_path: out_path.into(),
        overwrite: true,
        audio: None,
    }
}

/// Check whether `ffmpeg` appears to be available on `PATH`.
pub fn is_ffmpeg_on_path() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Create the parent directory for `path` if it does not exist.
pub fn ensure_parent_dir(path: &Path) -> WavyteResult<()> {
    if let Some(parent) = path.parent() {
        use anyhow::Context as _;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory '{}'", parent.display()))?;
    }
    Ok(())
}

/// Streaming MP4 encoder that wraps the system `ffmpeg` binary.
///
/// This encoder spawns `ffmpeg` and writes raw RGBA frames to stdin.
/// It is intentionally implemented without linking to FFmpeg libraries to avoid native dependency
/// requirements at compile time.
pub struct FfmpegEncoder {
    cfg: EncodeConfig,
    bg_rgba: [u8; 4],
    child: Child,
    stdin: Option<ChildStdin>,
    stderr_drain: Option<std::thread::JoinHandle<std::io::Result<Vec<u8>>>>,
    scratch: Vec<u8>,
}

impl FfmpegEncoder {
    /// Spawn `ffmpeg` and prepare to accept frames.
    pub fn new(cfg: EncodeConfig, bg_rgba: [u8; 4]) -> WavyteResult<Self> {
        cfg.validate()?;
        ensure_parent_dir(&cfg.out_path)?;

        if !cfg.overwrite && cfg.out_path.exists() {
            return Err(WavyteError::validation(format!(
                "output file '{}' already exists",
                cfg.out_path.display()
            )));
        }

        if !is_ffmpeg_on_path() {
            return Err(WavyteError::evaluation(
                "ffmpeg is required for MP4 encoding, but was not found on PATH",
            ));
        }

        // We intentionally use the system `ffmpeg` binary rather than `ffmpeg-next` to avoid
        // native FFmpeg dev header/lib requirements.
        let mut cmd = Command::new("ffmpeg");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        if cfg.overwrite {
            cmd.arg("-y");
        } else {
            cmd.arg("-n");
        }

        cmd.args([
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{}x{}", cfg.width, cfg.height),
            "-r",
            &cfg.fps.to_string(),
            "-i",
            "pipe:0",
        ]);
        if let Some(audio) = &cfg.audio {
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
        cmd.arg(&cfg.out_path);

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

        Ok(Self {
            scratch: vec![0u8; (cfg.width * cfg.height * 4) as usize],
            cfg,
            bg_rgba,
            child,
            stdin: Some(stdin),
            stderr_drain: Some(stderr_drain),
        })
    }

    /// Encode a single rendered frame.
    ///
    /// Wavyte renderers output premultiplied RGBA8 by default; this method can flatten either
    /// premultiplied or straight-alpha input over `bg_rgba`.
    pub fn encode_frame(&mut self, frame: &FrameRGBA) -> WavyteResult<()> {
        if frame.width != self.cfg.width || frame.height != self.cfg.height {
            return Err(WavyteError::validation(format!(
                "frame size mismatch: got {}x{}, expected {}x{}",
                frame.width, frame.height, self.cfg.width, self.cfg.height
            )));
        }

        if frame.data.len() != self.scratch.len() {
            return Err(WavyteError::validation(
                "frame.data size mismatch with width*height*4",
            ));
        }

        flatten_to_opaque_rgba8(
            &mut self.scratch,
            &frame.data,
            frame.premultiplied,
            self.bg_rgba,
        )?;

        let Some(stdin) = self.stdin.as_mut() else {
            return Err(WavyteError::evaluation(
                "ffmpeg encoder is already finalized",
            ));
        };

        use std::io::Write as _;
        stdin.write_all(&self.scratch).map_err(|e| {
            WavyteError::evaluation(format!("failed to write frame to ffmpeg stdin: {e}"))
        })?;

        Ok(())
    }

    /// Finalize the stream and wait for `ffmpeg` to exit.
    pub fn finish(mut self) -> WavyteResult<()> {
        drop(self.stdin.take());

        let status = self.child.wait().map_err(|e| {
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

        Ok(())
    }
}

fn flatten_to_opaque_rgba8(
    dst: &mut [u8],
    src: &[u8],
    src_is_premul: bool,
    bg_rgba: [u8; 4],
) -> WavyteResult<()> {
    if dst.len() != src.len() || !dst.len().is_multiple_of(4) {
        return Err(WavyteError::validation(
            "flatten_to_opaque_rgba8 expects equal-length rgba8 buffers",
        ));
    }

    let bg_r = bg_rgba[0] as u16;
    let bg_g = bg_rgba[1] as u16;
    let bg_b = bg_rgba[2] as u16;

    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let a = s[3] as u16;
        if a == 255 {
            d.copy_from_slice(s);
            d[3] = 255;
            continue;
        }

        let inv = 255u16 - a;

        let (r, g, b) = if src_is_premul {
            (
                s[0] as u16 + mul_div255(bg_r, inv),
                s[1] as u16 + mul_div255(bg_g, inv),
                s[2] as u16 + mul_div255(bg_b, inv),
            )
        } else {
            (
                mul_div255(s[0] as u16, a) + mul_div255(bg_r, inv),
                mul_div255(s[1] as u16, a) + mul_div255(bg_g, inv),
                mul_div255(s[2] as u16, a) + mul_div255(bg_b, inv),
            )
        };

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

#[cfg(test)]
#[path = "../../tests/unit/encode/ffmpeg.rs"]
mod tests;
