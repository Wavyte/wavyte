use super::*;
use crate::foundation::core::{Fps, FrameIndex};

fn ctx(frame: u64, seed: u64) -> SampleCtx {
    SampleCtx {
        frame: FrameIndex(frame),
        fps: Fps::new(30, 1).unwrap(),
        clip_local: FrameIndex(frame),
        seed,
    }
}

#[test]
fn rng_is_deterministic() {
    let mut a = Rng64::new(123);
    let mut b = Rng64::new(123);
    for _ in 0..10 {
        assert_eq!(a.next_u64(), b.next_u64());
    }
}

#[test]
fn noise_is_bounded_and_deterministic() {
    let proc = Procedural::<f64>::new(ProceduralKind::Scalar(ProcScalar::Noise1D {
        amp: 2.0,
        freq_hz: 1.0,
        offset: 0.5,
    }));
    let v0 = proc.sample(ctx(0, 7)).unwrap();
    let v1 = proc.sample(ctx(1, 7)).unwrap();
    assert_ne!(v0, v1);
    for v in [v0, v1] {
        assert!(v >= -1.5);
        assert!(v <= 2.5);
    }
    assert_eq!(v0, proc.sample(ctx(0, 7)).unwrap());
}

#[test]
fn envelope_basic_boundaries() {
    let proc = Procedural::<f64>::new(ProceduralKind::Scalar(ProcScalar::Envelope {
        attack: 10,
        decay: 10,
        sustain: 0.25,
        release: 10,
    }));
    assert_eq!(proc.sample(ctx(0, 0)).unwrap(), 0.0);
    assert!((proc.sample(ctx(10, 0)).unwrap() - 1.0).abs() < 1e-9);
    let at_sustain = proc.sample(ctx(20, 0)).unwrap();
    assert!((at_sustain - 0.25).abs() < 1e-9);
    let released = proc.sample(ctx(30, 0)).unwrap();
    assert!((released - 0.0).abs() < 1e-9);
}
