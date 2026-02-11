use super::*;

#[test]
fn frame_to_sample_uses_rational_fps() {
    let fps = Fps::new(30000, 1001).unwrap();
    let samples = frame_to_sample(300, fps, 48_000);
    assert!(samples > 470_000 && samples < 490_000);
}

#[test]
fn mix_applies_overlap_and_fades() {
    let seg_a = AudioSegment {
        timeline_start_sample: 0,
        timeline_end_sample: 4,
        source_start_sec: 0.0,
        source_end_sec: None,
        playback_rate: 1.0,
        volume: 1.0,
        fade_in_sec: 0.0,
        fade_out_sec: 0.0,
        source_sample_rate: 4,
        source_channels: 2,
        source_interleaved_f32: Arc::new(vec![0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25]),
    };
    let seg_b = AudioSegment {
        timeline_start_sample: 2,
        timeline_end_sample: 4,
        source_start_sec: 0.0,
        source_end_sec: None,
        playback_rate: 1.0,
        volume: 1.0,
        fade_in_sec: 0.5,
        fade_out_sec: 0.0,
        source_sample_rate: 4,
        source_channels: 2,
        source_interleaved_f32: Arc::new(vec![1.0, 1.0, 1.0, 1.0]),
    };

    let manifest = AudioManifest {
        sample_rate: 4,
        channels: 2,
        total_samples: 4,
        segments: vec![seg_a, seg_b],
    };
    let out = mix_manifest(&manifest);
    assert_eq!(out.len(), 8);
    assert!((out[0] - 0.25).abs() < 1e-6);
    assert!(out[4] >= 0.25);
    assert!(out[6] > out[4]);
}
