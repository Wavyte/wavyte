use super::*;
use crate::foundation::core::Fps;

fn ctx(frame: u64) -> SampleCtx {
    SampleCtx {
        frame: FrameIndex(frame),
        fps: Fps::new(30, 1).unwrap(),
        clip_local: FrameIndex(frame),
        seed: 0,
    }
}

#[test]
fn keyframes_hold_is_constant_between_keys() {
    let anim = Anim::Keyframes(Keyframes {
        keys: vec![
            Keyframe {
                frame: FrameIndex(0),
                value: 1.0,
                ease: Ease::Linear,
            },
            Keyframe {
                frame: FrameIndex(10),
                value: 3.0,
                ease: Ease::Linear,
            },
        ],
        mode: InterpMode::Hold,
        default: None,
    });
    assert_eq!(anim.sample(ctx(5)).unwrap(), 1.0);
    assert_eq!(anim.sample(ctx(10)).unwrap(), 3.0);
}

#[test]
fn keyframes_linear_interpolates() {
    let anim = Anim::Keyframes(Keyframes {
        keys: vec![
            Keyframe {
                frame: FrameIndex(0),
                value: 0.0,
                ease: Ease::Linear,
            },
            Keyframe {
                frame: FrameIndex(10),
                value: 10.0,
                ease: Ease::Linear,
            },
        ],
        mode: InterpMode::Linear,
        default: None,
    });
    assert_eq!(anim.sample(ctx(5)).unwrap(), 5.0);
}

#[test]
fn expr_reverse_maps_frames() {
    let inner = Anim::Keyframes(Keyframes {
        keys: vec![
            Keyframe {
                frame: FrameIndex(0),
                value: 0.0,
                ease: Ease::Linear,
            },
            Keyframe {
                frame: FrameIndex(9),
                value: 9.0,
                ease: Ease::Linear,
            },
        ],
        mode: InterpMode::Hold,
        default: None,
    });
    let rev = Anim::Expr(Expr::Reverse {
        inner: Box::new(inner),
        duration: 10,
    });
    assert_eq!(rev.sample(ctx(0)).unwrap(), 9.0);
    assert_eq!(rev.sample(ctx(9)).unwrap(), 0.0);
}
