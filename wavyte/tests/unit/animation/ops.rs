use super::*;
use crate::animation::anim::{InterpMode, Keyframe, Keyframes, SampleCtx};
use crate::foundation::core::{Fps, FrameIndex};

fn ctx(frame: u64) -> SampleCtx {
    SampleCtx {
        frame: FrameIndex(frame),
        fps: Fps::new(30, 1).unwrap(),
        clip_local: FrameIndex(frame),
        seed: 0,
    }
}

#[test]
fn sequence_switches_at_boundary() {
    let a = Anim::constant(1.0);
    let b = Anim::Keyframes(Keyframes {
        keys: vec![Keyframe {
            frame: FrameIndex(0),
            value: 10.0,
            ease: crate::animation::ease::Ease::Linear,
        }],
        mode: InterpMode::Hold,
        default: None,
    });

    let s = sequence(a, 5, b);
    assert_eq!(s.sample(ctx(4)).unwrap(), 1.0);
    assert_eq!(s.sample(ctx(5)).unwrap(), 10.0);
}
