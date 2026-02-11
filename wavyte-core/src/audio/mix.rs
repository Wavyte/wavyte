use std::{path::Path, sync::Arc};

use crate::{
    assets::media,
    assets::store::{PreparedAsset, PreparedAssetStore},
    composition::model::{Asset, AudioAsset, Clip, Composition, VideoAsset},
    foundation::core::{Fps, FrameIndex, FrameRange},
    foundation::error::{WavyteError, WavyteResult},
};

#[derive(Clone, Debug)]
pub struct AudioSegment {
    pub timeline_start_sample: u64,
    pub timeline_end_sample: u64,
    pub source_start_sec: f64,
    pub source_end_sec: Option<f64>,
    pub playback_rate: f64,
    pub volume: f32,
    pub fade_in_sec: f64,
    pub fade_out_sec: f64,
    pub source_sample_rate: u32,
    pub source_channels: u16,
    pub source_interleaved_f32: Arc<Vec<f32>>,
}

#[derive(Clone, Debug)]
pub struct AudioManifest {
    pub sample_rate: u32,
    pub channels: u16,
    pub total_samples: u64,
    pub segments: Vec<AudioSegment>,
}

pub fn build_audio_manifest(
    comp: &Composition,
    assets: &PreparedAssetStore,
    range: FrameRange,
) -> WavyteResult<AudioManifest> {
    if range.is_empty() {
        return Err(WavyteError::validation(
            "audio manifest range must be non-empty",
        ));
    }

    let sample_rate = media::MIX_SAMPLE_RATE;
    let mut segments = Vec::<AudioSegment>::new();
    for track in &comp.tracks {
        for clip in &track.clips {
            if let Some(intersection) = intersect_ranges(clip.range, range) {
                let asset = comp.assets.get(&clip.asset).ok_or_else(|| {
                    WavyteError::evaluation(format!(
                        "clip references missing asset '{}'",
                        clip.asset
                    ))
                })?;
                match asset {
                    Asset::Audio(audio_asset) => {
                        push_audio_segment(
                            &mut segments,
                            clip,
                            &intersection,
                            range.start,
                            comp.fps,
                            audio_asset,
                            assets,
                        )?;
                    }
                    Asset::Video(video_asset) => {
                        push_video_audio_segment(
                            &mut segments,
                            clip,
                            &intersection,
                            range.start,
                            comp.fps,
                            video_asset,
                            assets,
                        )?;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(AudioManifest {
        sample_rate,
        channels: 2,
        total_samples: frame_to_sample(range.len_frames(), comp.fps, sample_rate),
        segments,
    })
}

pub fn mix_manifest(manifest: &AudioManifest) -> Vec<f32> {
    let frames = manifest.total_samples as usize;
    let mut out = vec![0.0f32; frames * usize::from(manifest.channels)];

    for seg in &manifest.segments {
        let seg_len_samples = seg
            .timeline_end_sample
            .saturating_sub(seg.timeline_start_sample);
        if seg_len_samples == 0 {
            continue;
        }
        let src = seg.source_interleaved_f32.as_ref();
        let src_frames = src.len() / usize::from(seg.source_channels);
        if src_frames == 0 {
            continue;
        }

        for dst_sample in seg.timeline_start_sample..seg.timeline_end_sample {
            let rel_sample = dst_sample - seg.timeline_start_sample;
            let rel_sec = (rel_sample as f64) / f64::from(manifest.sample_rate);
            let src_sec = seg.source_start_sec + rel_sec * seg.playback_rate;
            if let Some(end_sec) = seg.source_end_sec
                && src_sec >= end_sec
            {
                break;
            }
            let src_pos = src_sec * f64::from(seg.source_sample_rate);
            if !src_pos.is_finite() || src_pos < 0.0 {
                break;
            }
            let src_frame0 = src_pos.floor() as usize;
            if src_frame0 >= src_frames {
                break;
            }
            let src_frame1 = (src_frame0 + 1).min(src_frames.saturating_sub(1));
            let frac = (src_pos - src_frame0 as f64) as f32;

            let src_gain = fade_gain(seg, rel_sec, seg_len_samples, manifest.sample_rate);
            let gain = src_gain * seg.volume;
            let dst_idx = dst_sample as usize * usize::from(manifest.channels);
            let (l, r) = if seg.source_channels == 1 {
                let v0 = src[src_frame0];
                let v1 = src[src_frame1];
                let v = v0 + ((v1 - v0) * frac);
                (v, v)
            } else {
                let i0 = src_frame0 * usize::from(seg.source_channels);
                let i1 = src_frame1 * usize::from(seg.source_channels);
                let l0 = src[i0];
                let l1 = src[i1];
                let r0 = src[i0 + 1];
                let r1 = src[i1 + 1];
                (l0 + ((l1 - l0) * frac), r0 + ((r1 - r0) * frac))
            };

            out[dst_idx] += l * gain;
            if manifest.channels > 1 {
                out[dst_idx + 1] += r * gain;
            }
        }
    }

    for s in &mut out {
        *s = s.clamp(-1.0, 1.0);
    }
    out
}

pub fn write_mix_to_f32le_file(samples_interleaved: &[f32], out_path: &Path) -> WavyteResult<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            WavyteError::evaluation(format!(
                "failed to create audio mix output directory '{}': {e}",
                parent.display()
            ))
        })?;
    }

    let mut bytes = Vec::<u8>::with_capacity(samples_interleaved.len() * 4);
    for &sample in samples_interleaved {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(out_path, bytes).map_err(|e| {
        WavyteError::evaluation(format!(
            "failed to write mixed audio file '{}': {e}",
            out_path.display()
        ))
    })
}

fn fade_gain(seg: &AudioSegment, rel_sec: f64, seg_len_samples: u64, sample_rate: u32) -> f32 {
    let mut gain = 1.0f32;
    if seg.fade_in_sec > 0.0 {
        let t = (rel_sec / seg.fade_in_sec).clamp(0.0, 1.0) as f32;
        gain *= t;
    }
    if seg.fade_out_sec > 0.0 {
        let seg_len_sec = (seg_len_samples as f64) / f64::from(sample_rate);
        let rem = (seg_len_sec - rel_sec).max(0.0);
        let t = (rem / seg.fade_out_sec).clamp(0.0, 1.0) as f32;
        gain *= t;
    }
    gain
}

fn push_audio_segment(
    out: &mut Vec<AudioSegment>,
    clip: &Clip,
    intersection: &FrameRange,
    range_start: FrameIndex,
    fps: Fps,
    audio_asset: &AudioAsset,
    assets: &PreparedAssetStore,
) -> WavyteResult<()> {
    if audio_asset.muted || audio_asset.volume <= 0.0 {
        return Ok(());
    }
    let id = assets.id_for_key(&clip.asset)?;
    let prepared = assets.get(id)?;
    let PreparedAsset::Audio(pcm) = prepared else {
        return Err(WavyteError::evaluation(
            "audio clip references non-audio prepared asset",
        ));
    };
    push_segment_common(
        out,
        intersection,
        range_start,
        clip.range.start,
        fps,
        media::audio_source_time_sec(audio_asset, intersection.start.0 - clip.range.start.0, fps),
        audio_asset.trim_end_sec,
        audio_asset.playback_rate,
        audio_asset.volume as f32,
        audio_asset.fade_in_sec,
        audio_asset.fade_out_sec,
        pcm.sample_rate,
        pcm.channels,
        pcm.interleaved_f32.clone(),
    );
    Ok(())
}

fn push_video_audio_segment(
    out: &mut Vec<AudioSegment>,
    clip: &Clip,
    intersection: &FrameRange,
    range_start: FrameIndex,
    fps: Fps,
    video_asset: &VideoAsset,
    assets: &PreparedAssetStore,
) -> WavyteResult<()> {
    if video_asset.muted || video_asset.volume <= 0.0 {
        return Ok(());
    }
    let id = assets.id_for_key(&clip.asset)?;
    let prepared = assets.get(id)?;
    let PreparedAsset::Video(video) = prepared else {
        return Err(WavyteError::evaluation(
            "video clip references non-video prepared asset",
        ));
    };
    let Some(audio) = &video.audio else {
        return Ok(());
    };
    push_segment_common(
        out,
        intersection,
        range_start,
        clip.range.start,
        fps,
        media::video_source_time_sec(video_asset, intersection.start.0 - clip.range.start.0, fps),
        video_asset.trim_end_sec,
        video_asset.playback_rate,
        video_asset.volume as f32,
        video_asset.fade_in_sec,
        video_asset.fade_out_sec,
        audio.sample_rate,
        audio.channels,
        audio.interleaved_f32.clone(),
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_segment_common(
    out: &mut Vec<AudioSegment>,
    intersection: &FrameRange,
    range_start: FrameIndex,
    clip_start: FrameIndex,
    fps: Fps,
    source_start_sec: f64,
    source_end_sec: Option<f64>,
    playback_rate: f64,
    volume: f32,
    fade_in_sec: f64,
    fade_out_sec: f64,
    source_sample_rate: u32,
    source_channels: u16,
    source_interleaved_f32: Arc<Vec<f32>>,
) {
    let timeline_start_sample = frame_to_sample(
        intersection.start.0 - range_start.0,
        fps,
        media::MIX_SAMPLE_RATE,
    );
    let timeline_end_sample = frame_to_sample(
        intersection.end.0 - range_start.0,
        fps,
        media::MIX_SAMPLE_RATE,
    );

    let _ = clip_start;
    out.push(AudioSegment {
        timeline_start_sample,
        timeline_end_sample,
        source_start_sec,
        source_end_sec,
        playback_rate,
        volume,
        fade_in_sec,
        fade_out_sec,
        source_sample_rate,
        source_channels,
        source_interleaved_f32,
    });
}

pub fn frame_to_sample(frame_delta: u64, fps: Fps, sample_rate: u32) -> u64 {
    let num = u128::from(frame_delta) * u128::from(sample_rate) * u128::from(fps.den);
    let den = u128::from(fps.num);
    ((num + (den / 2)) / den) as u64
}

fn intersect_ranges(a: FrameRange, b: FrameRange) -> Option<FrameRange> {
    let start = a.start.0.max(b.start.0);
    let end = a.end.0.min(b.end.0);
    if start >= end {
        return None;
    }
    FrameRange::new(FrameIndex(start), FrameIndex(end)).ok()
}

#[cfg(test)]
#[path = "../../tests/unit/audio/mix.rs"]
mod tests;
