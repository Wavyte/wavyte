use std::path::{Path, PathBuf};

use crate::{
    error::{WavyteError, WavyteResult},
    model::{AudioAsset, VideoAsset},
};

pub const MIX_SAMPLE_RATE: u32 = 48_000;

#[derive(Clone, Debug)]
pub struct VideoSourceInfo {
    pub source_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub duration_sec: f64,
    pub has_audio: bool,
}

#[derive(Clone, Debug)]
pub struct AudioPcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub interleaved_f32: Vec<f32>,
}

impl VideoSourceInfo {
    pub fn source_fps(&self) -> f64 {
        if self.fps_den == 0 {
            0.0
        } else {
            f64::from(self.fps_num) / f64::from(self.fps_den)
        }
    }
}

pub fn video_source_time_sec(asset: &VideoAsset, clip_local_frames: u64, fps: crate::Fps) -> f64 {
    let timeline_t = (clip_local_frames as f64) * (f64::from(fps.den) / f64::from(fps.num));
    let mut src_t = asset.trim_start_sec + timeline_t * asset.playback_rate;
    if let Some(end) = asset.trim_end_sec {
        src_t = src_t.min(end.max(asset.trim_start_sec));
    }
    src_t.max(0.0)
}

pub fn audio_source_time_sec(asset: &AudioAsset, clip_local_frames: u64, fps: crate::Fps) -> f64 {
    let timeline_t = (clip_local_frames as f64) * (f64::from(fps.den) / f64::from(fps.num));
    let mut src_t = asset.trim_start_sec + timeline_t * asset.playback_rate;
    if let Some(end) = asset.trim_end_sec {
        src_t = src_t.min(end.max(asset.trim_start_sec));
    }
    src_t.max(0.0)
}

#[cfg(feature = "media-ffmpeg")]
pub fn probe_video(source_path: &Path) -> WavyteResult<VideoSourceInfo> {
    #[derive(serde::Deserialize)]
    struct ProbeStream {
        codec_type: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
        r_frame_rate: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct ProbeFormat {
        duration: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct ProbeOut {
        streams: Vec<ProbeStream>,
        format: Option<ProbeFormat>,
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

    let (fps_num, fps_den) = parse_ff_ratio(video_stream.r_frame_rate.as_deref().unwrap_or("0/1"))
        .ok_or_else(|| WavyteError::evaluation("invalid video r_frame_rate"))?;
    let duration_sec = parsed
        .format
        .as_ref()
        .and_then(|f| f.duration.as_ref())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let has_audio = parsed
        .streams
        .iter()
        .any(|s| s.codec_type.as_deref() == Some("audio"));

    Ok(VideoSourceInfo {
        source_path: source_path.to_path_buf(),
        width,
        height,
        fps_num,
        fps_den,
        duration_sec,
        has_audio,
    })
}

#[cfg(not(feature = "media-ffmpeg"))]
pub fn probe_video(_source_path: &Path) -> WavyteResult<VideoSourceInfo> {
    Err(WavyteError::evaluation(
        "video/audio assets require the 'media-ffmpeg' feature",
    ))
}

#[cfg(feature = "media-ffmpeg")]
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
pub fn decode_video_frame_rgba8(
    _source: &VideoSourceInfo,
    _source_time_sec: f64,
) -> WavyteResult<Vec<u8>> {
    Err(WavyteError::evaluation(
        "video/audio assets require the 'media-ffmpeg' feature",
    ))
}

#[cfg(not(feature = "media-ffmpeg"))]
pub(crate) fn decode_video_frames_rgba8(
    _source: &VideoSourceInfo,
    _start_time_sec: f64,
    _frame_count: u32,
) -> WavyteResult<Vec<Vec<u8>>> {
    Err(WavyteError::evaluation(
        "video/audio assets require the 'media-ffmpeg' feature",
    ))
}

#[cfg(feature = "media-ffmpeg")]
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
pub fn decode_audio_f32_stereo(_path: &Path, _sample_rate: u32) -> WavyteResult<AudioPcm> {
    Err(WavyteError::evaluation(
        "video/audio assets require the 'media-ffmpeg' feature",
    ))
}

#[cfg(feature = "media-ffmpeg")]
fn parse_ff_ratio(s: &str) -> Option<(u32, u32)> {
    let mut parts = s.split('/');
    let a = parts.next()?.parse::<u32>().ok()?;
    let b = parts.next()?.parse::<u32>().ok()?;
    if b == 0 {
        return None;
    }
    Some((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_time_mapping_applies_trim_and_rate() {
        let video = VideoAsset {
            source: "a.mp4".to_string(),
            trim_start_sec: 1.0,
            trim_end_sec: Some(10.0),
            playback_rate: 2.0,
            volume: 1.0,
            fade_in_sec: 0.0,
            fade_out_sec: 0.0,
            muted: false,
        };

        let t = video_source_time_sec(&video, 15, crate::Fps::new(30, 1).unwrap());
        assert!((t - 2.0).abs() < 1e-9);
    }
}
