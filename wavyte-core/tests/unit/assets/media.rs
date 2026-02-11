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
