use std::path::{Path, PathBuf};

use crate::error::{WavyteError, WavyteResult};

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
}
