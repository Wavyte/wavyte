use std::path::{Path, PathBuf};

use crate::foundation::error::{WavyteError, WavyteResult};

/// Internal audio mixing sample rate used across decode/mix/encode pipeline.
pub const MIX_SAMPLE_RATE: u32 = 48_000;

#[derive(Clone, Debug)]
/// Basic metadata about a source video file.
pub struct VideoSourceInfo {
    /// Absolute source path used for probing/decoding.
    #[cfg_attr(not(feature = "media-ffmpeg"), allow(dead_code))]
    pub source_path: PathBuf,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Whether ffprobe detected at least one audio stream.
    pub has_audio: bool,
}

#[derive(Clone, Debug)]
/// Decoded interleaved floating-point PCM.
pub struct AudioPcm {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Interleaved `f32` PCM samples.
    pub interleaved_f32: Vec<f32>,
}

/// Probe source video metadata through `ffprobe`.
#[cfg(feature = "media-ffmpeg")]
pub fn probe_video(source_path: &Path) -> WavyteResult<VideoSourceInfo> {
    #[derive(serde::Deserialize)]
    struct ProbeStream {
        codec_type: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
    }
    #[derive(serde::Deserialize)]
    struct ProbeOut {
        streams: Vec<ProbeStream>,
    }

    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(source_path)
        .output()
        .map_err(|e| WavyteError::evaluation(format!("failed to run ffprobe: {e}")))?;
    if !out.status.success() {
        return Err(WavyteError::evaluation(format!(
            "ffprobe failed for '{}': {}",
            source_path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    let parsed: ProbeOut = serde_json::from_slice(&out.stdout)
        .map_err(|e| WavyteError::evaluation(format!("ffprobe json parse failed: {e}")))?;
    let video_stream = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .ok_or_else(|| WavyteError::evaluation("no video stream found"))?;
    let width = video_stream
        .width
        .ok_or_else(|| WavyteError::evaluation("missing video width from ffprobe"))?;
    let height = video_stream
        .height
        .ok_or_else(|| WavyteError::evaluation("missing video height from ffprobe"))?;
    let has_audio = parsed
        .streams
        .iter()
        .any(|s| s.codec_type.as_deref() == Some("audio"));

    Ok(VideoSourceInfo {
        source_path: source_path.to_path_buf(),
        width,
        height,
        has_audio,
    })
}

#[cfg(not(feature = "media-ffmpeg"))]
/// Probe source video metadata through `ffprobe`.
///
/// Returns an error when `media-ffmpeg` feature is disabled.
pub fn probe_video(_source_path: &Path) -> WavyteResult<VideoSourceInfo> {
    Err(WavyteError::evaluation(
        "video/audio assets require the 'media-ffmpeg' feature",
    ))
}

#[cfg(feature = "media-ffmpeg")]
/// Decode a single RGBA frame from source video at `source_time_sec`.
pub fn decode_video_frame_rgba8(
    source: &VideoSourceInfo,
    source_time_sec: f64,
) -> WavyteResult<Vec<u8>> {
    let mut frames = decode_video_frames_rgba8(source, source_time_sec, 1)?;
    frames.pop().ok_or_else(|| {
        WavyteError::evaluation(format!(
            "ffmpeg returned no video frames for '{}'",
            source.source_path.display()
        ))
    })
}

#[cfg(feature = "media-ffmpeg")]
/// Decode up to `frame_count` sequential RGBA frames from source video.
pub(crate) fn decode_video_frames_rgba8(
    source: &VideoSourceInfo,
    start_time_sec: f64,
    frame_count: u32,
) -> WavyteResult<Vec<Vec<u8>>> {
    if frame_count == 0 {
        return Ok(Vec::new());
    }

    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &format!("{start_time_sec:.9}")])
        .arg("-i")
        .arg(&source.source_path)
        .args([
            "-frames:v",
            &frame_count.to_string(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ])
        .output()
        .map_err(|e| {
            WavyteError::evaluation(format!("failed to run ffmpeg for video decode: {e}"))
        })?;

    if !out.status.success() {
        return Err(WavyteError::evaluation(format!(
            "ffmpeg video decode batch failed for '{}': {}",
            source.source_path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    let expected_len = source.width as usize * source.height as usize * 4;
    if expected_len == 0 {
        return Err(WavyteError::evaluation(
            "decoded video frame size is zero (invalid source dimensions)",
        ));
    }
    if out.stdout.len() < expected_len || !out.stdout.len().is_multiple_of(expected_len) {
        return Err(WavyteError::evaluation(format!(
            "decoded video batch has invalid size: got {} bytes, expected multiples of {expected_len}",
            out.stdout.len()
        )));
    }

    let available = (out.stdout.len() / expected_len).min(frame_count as usize);
    let mut frames = Vec::with_capacity(available);
    for idx in 0..available {
        let off = idx * expected_len;
        frames.push(out.stdout[off..off + expected_len].to_vec());
    }
    Ok(frames)
}

#[cfg(not(feature = "media-ffmpeg"))]
/// Decode a single RGBA frame from source video at `source_time_sec`.
///
/// Returns an error when `media-ffmpeg` feature is disabled.
pub fn decode_video_frame_rgba8(
    _source: &VideoSourceInfo,
    _source_time_sec: f64,
) -> WavyteResult<Vec<u8>> {
    Err(WavyteError::evaluation(
        "video/audio assets require the 'media-ffmpeg' feature",
    ))
}

#[cfg(feature = "media-ffmpeg")]
/// Decode audio from media source to stereo interleaved `f32` PCM.
pub fn decode_audio_f32_stereo(path: &Path, sample_rate: u32) -> WavyteResult<AudioPcm> {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-vn",
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "-ac",
            "2",
            "-ar",
            &sample_rate.to_string(),
            "pipe:1",
        ])
        .output()
        .map_err(|e| {
            WavyteError::evaluation(format!("failed to run ffmpeg for audio decode: {e}"))
        })?;

    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        // ffmpeg reports no audio stream with an error. Treat this as empty PCM for video files
        // without audio tracks.
        if msg.contains("Stream specifier")
            || msg.contains("matches no streams")
            || msg.contains("Output file #0 does not contain any stream")
        {
            return Ok(AudioPcm {
                sample_rate,
                channels: 2,
                interleaved_f32: Vec::new(),
            });
        }
        return Err(WavyteError::evaluation(format!(
            "ffmpeg audio decode failed for '{}': {}",
            path.display(),
            msg.trim()
        )));
    }

    if !out.stdout.len().is_multiple_of(4) {
        return Err(WavyteError::evaluation(
            "decoded audio byte length is not aligned to f32 samples",
        ));
    }
    let mut pcm = Vec::<f32>::with_capacity(out.stdout.len() / 4);
    for chunk in out.stdout.chunks_exact(4) {
        pcm.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }

    Ok(AudioPcm {
        sample_rate,
        channels: 2,
        interleaved_f32: pcm,
    })
}

#[cfg(not(feature = "media-ffmpeg"))]
/// Decode audio from media source to stereo interleaved `f32` PCM.
///
/// Returns an error when `media-ffmpeg` feature is disabled.
pub fn decode_audio_f32_stereo(_path: &Path, _sample_rate: u32) -> WavyteResult<AudioPcm> {
    Err(WavyteError::evaluation(
        "video/audio assets require the 'media-ffmpeg' feature",
    ))
}

// No unit tests here: these functions shell out to `ffprobe`/`ffmpeg` and are best validated via
// integration tests that can be conditionally ignored when the tools are unavailable.
