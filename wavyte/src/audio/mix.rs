use crate::audio::manifest::{AudioManifest, AudioSegment};
use crate::foundation::core::Fps;
use crate::foundation::error::{WavyteError, WavyteResult};
use std::path::Path;

/// Mix all manifest segments into interleaved output PCM.
pub(crate) fn mix_manifest(manifest: &AudioManifest) -> Vec<f32> {
    let frames = manifest.total_samples as usize;
    let mut out = vec![0.0f32; frames * usize::from(manifest.channels)];

    for seg in &manifest.segments {
        mix_segment(&mut out, manifest, seg);
    }

    for s in &mut out {
        *s = s.clamp(-1.0, 1.0);
    }
    out
}

fn mix_segment(out: &mut [f32], manifest: &AudioManifest, seg: &AudioSegment) {
    let seg_len_samples = seg
        .timeline_end_sample
        .saturating_sub(seg.timeline_start_sample);
    if seg_len_samples == 0 {
        return;
    }

    let src = seg.source_interleaved_f32.as_ref();
    let src_frames = src.len() / usize::from(seg.source_channels);
    if src_frames == 0 {
        return;
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

/// Write interleaved `f32` PCM samples to raw little-endian `.f32le` file.
pub(crate) fn write_mix_to_f32le_file(
    samples_interleaved: &[f32],
    out_path: &Path,
) -> WavyteResult<()> {
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

/// Convert a frame delta to the nearest sample index at `sample_rate`.
pub(crate) fn frame_to_sample(frame_delta: u64, fps: Fps, sample_rate: u32) -> u64 {
    let num = u128::from(frame_delta) * u128::from(sample_rate) * u128::from(fps.den);
    let den = u128::from(fps.num);
    ((num + (den / 2)) / den) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::manifest::AudioManifest;
    use std::sync::Arc;

    #[test]
    fn frame_to_sample_uses_rational_fps() {
        // 30000/1001 ~ 29.97
        let fps = Fps {
            num: 30_000,
            den: 1001,
        };
        let s0 = frame_to_sample(0, fps, 48_000);
        let s1 = frame_to_sample(1, fps, 48_000);
        assert_eq!(s0, 0);
        assert!(s1 > 0);
    }

    #[test]
    fn mix_applies_fade_in() {
        let manifest = AudioManifest {
            sample_rate: 48_000,
            channels: 2,
            total_samples: 48_000,
            segments: vec![AudioSegment {
                timeline_start_sample: 0,
                timeline_end_sample: 48_000,
                source_start_sec: 0.0,
                source_end_sec: None,
                playback_rate: 1.0,
                volume: 1.0,
                fade_in_sec: 1.0,
                fade_out_sec: 0.0,
                source_sample_rate: 48_000,
                source_channels: 2,
                source_interleaved_f32: Arc::new(vec![1.0; 48_000 * 2]),
            }],
        };
        let out = mix_manifest(&manifest);
        // First sample is faded in (gain 0), last sample is ~1.0.
        assert!(out[0].abs() < 1e-6);
        assert!(out[out.len() - 2] > 0.5);
    }
}
