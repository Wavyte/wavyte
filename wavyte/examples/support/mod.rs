use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context as _;

use wavyte::{
    BackendKind, Composition, FrameIndex, FrameRange, RenderSettings, RenderThreading,
    RenderToMp4Opts, create_backend, render_to_mp4,
};

pub fn ffmpeg_tools_available() -> bool {
    let ffmpeg_ok = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let ffprobe_ok = Command::new("ffprobe")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ffmpeg_ok && ffprobe_ok
}

pub fn ensure_ffmpeg_tools() -> anyhow::Result<()> {
    if ffmpeg_tools_available() {
        return Ok(());
    }
    anyhow::bail!("ffmpeg + ffprobe are required and must be on PATH");
}

pub fn render_comp_to_assets_mp4(
    comp: &Composition,
    out_name: &str,
    clear_rgba: [u8; 4],
    threading: RenderThreading,
) -> anyhow::Result<PathBuf> {
    comp.validate()?;
    ensure_ffmpeg_tools()?;

    let out_dir = PathBuf::from("assets");
    std::fs::create_dir_all(&out_dir).context("create assets/ output directory")?;
    let out_path = out_dir.join(out_name);

    let settings = RenderSettings {
        clear_rgba: Some(clear_rgba),
    };
    let mut backend = create_backend(BackendKind::Cpu, &settings)?;
    let assets = wavyte::PreparedAssetStore::prepare(comp, ".")?;

    render_to_mp4(
        comp,
        &out_path,
        RenderToMp4Opts {
            range: FrameRange::new(FrameIndex(0), comp.duration)?,
            bg_rgba: clear_rgba,
            overwrite: true,
            threading,
        },
        backend.as_mut(),
        &assets,
    )?;

    Ok(out_path)
}

#[allow(dead_code)]
pub fn ensure_test_video_mp4(video_path: &Path, fallback_image: &Path) -> anyhow::Result<()> {
    if video_path.exists() {
        return Ok(());
    }

    ensure_ffmpeg_tools()?;
    if !fallback_image.exists() {
        anyhow::bail!(
            "missing '{}' and fallback image '{}'",
            video_path.display(),
            fallback_image.display()
        );
    }

    if let Some(parent) = video_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create directory '{}'", parent.display()))?;
    }

    let duration_s = "5";
    let vf = "scale=960:540:force_original_aspect_ratio=decrease,pad=960:540:(ow-iw)/2:(oh-ih)/2,format=yuv420p";
    let status = Command::new("ffmpeg")
        .arg("-v")
        .arg("error")
        .arg("-y")
        .arg("-loop")
        .arg("1")
        .arg("-i")
        .arg(fallback_image)
        .arg("-t")
        .arg(duration_s)
        .arg("-vf")
        .arg(vf)
        .arg("-r")
        .arg("30")
        .arg("-c:v")
        .arg("libx264")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg(video_path)
        .status()
        .with_context(|| format!("spawn ffmpeg to create '{}'", video_path.display()))?;

    if !status.success() {
        anyhow::bail!("ffmpeg failed creating '{}'", video_path.display());
    }

    Ok(())
}
