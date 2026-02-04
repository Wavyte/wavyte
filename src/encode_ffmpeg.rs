use std::{
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
};

use crate::{
    error::{WavyteError, WavyteResult},
    render::FrameRGBA,
};

#[derive(Clone, Debug)]
pub struct EncodeConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub out_path: PathBuf,
    pub overwrite: bool,
}

impl EncodeConfig {
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
        Ok(())
    }

    pub fn with_out_path(mut self, out_path: impl Into<PathBuf>) -> Self {
        self.out_path = out_path.into();
        self
    }
}

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
    }
}

pub fn is_ffmpeg_on_path() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn ensure_parent_dir(path: &Path) -> WavyteResult<()> {
    if let Some(parent) = path.parent() {
        use anyhow::Context as _;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory '{}'", parent.display()))?;
    }
    Ok(())
}

pub struct FfmpegEncoder {
    cfg: EncodeConfig,
    bg_rgba: [u8; 4],
    child: Child,
    stdin: Option<ChildStdin>,
    scratch: Vec<u8>,
}

impl FfmpegEncoder {
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
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ])
        .arg(&cfg.out_path);

        let mut child = cmd.spawn().map_err(|e| {
            WavyteError::evaluation(format!(
                "failed to spawn ffmpeg (is it installed and on PATH?): {e}"
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| WavyteError::evaluation("failed to open ffmpeg stdin (unexpected)"))?;

        Ok(Self {
            scratch: vec![0u8; (cfg.width * cfg.height * 4) as usize],
            cfg,
            bg_rgba,
            child,
            stdin: Some(stdin),
        })
    }

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

    pub fn finish(mut self) -> WavyteResult<()> {
        drop(self.stdin.take());

        let output = self.child.wait_with_output().map_err(|e| {
            WavyteError::evaluation(format!("failed to wait for ffmpeg to finish: {e}"))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(WavyteError::evaluation(format!(
                "ffmpeg exited with status {}: {}",
                output.status,
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
    (((u32::from(x) * u32::from(y)) + 127) / 255) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_validation_catches_bad_values() {
        assert!(
            EncodeConfig {
                width: 0,
                height: 10,
                fps: 30,
                out_path: PathBuf::from("assets/out.mp4"),
                overwrite: true,
            }
            .validate()
            .is_err()
        );

        assert!(
            EncodeConfig {
                width: 11,
                height: 10,
                fps: 30,
                out_path: PathBuf::from("assets/out.mp4"),
                overwrite: true,
            }
            .validate()
            .is_err()
        );

        assert!(
            EncodeConfig {
                width: 10,
                height: 10,
                fps: 0,
                out_path: PathBuf::from("assets/out.mp4"),
                overwrite: true,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn flatten_premul_over_black_produces_expected_rgb() {
        // Premultiplied red @ 50% alpha => rgb is 128,0,0 when premul.
        let src = vec![128u8, 0u8, 0u8, 128u8];
        let mut dst = vec![0u8; 4];
        flatten_to_opaque_rgba8(&mut dst, &src, true, [0, 0, 0, 255]).unwrap();
        assert_eq!(dst, vec![128u8, 0u8, 0u8, 255u8]);
    }

    #[test]
    fn flatten_straight_over_black_produces_expected_rgb() {
        // Straight red @ 50% alpha => rgb becomes 128,0,0 over black.
        let src = vec![255u8, 0u8, 0u8, 128u8];
        let mut dst = vec![0u8; 4];
        flatten_to_opaque_rgba8(&mut dst, &src, false, [0, 0, 0, 255]).unwrap();
        assert_eq!(dst, vec![128u8, 0u8, 0u8, 255u8]);
    }
}
