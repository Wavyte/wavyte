use super::*;

#[test]
fn frame_range_contains_boundaries() {
    let r = FrameRange::new(FrameIndex(2), FrameIndex(5)).unwrap();
    assert!(!r.contains(FrameIndex(1)));
    assert!(r.contains(FrameIndex(2)));
    assert!(r.contains(FrameIndex(4)));
    assert!(!r.contains(FrameIndex(5)));
}

#[test]
fn fps_frames_secs_roundtrip_floor() {
    let fps = Fps::new(30000, 1001).unwrap();
    let secs = fps.frames_to_secs(123);
    assert_eq!(fps.secs_to_frames_floor(secs), 123);
}

#[test]
fn transform_to_affine_identity_and_translation() {
    let t = Transform2D::default();
    assert_eq!(t.to_affine(), kurbo::Affine::IDENTITY);

    let t = Transform2D {
        translate: Vec2::new(10.0, -2.5),
        ..Transform2D::default()
    };
    assert_eq!(
        t.to_affine(),
        kurbo::Affine::translate(Vec2::new(10.0, -2.5))
    );
}
