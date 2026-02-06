use crate::anim::{Anim, Expr, LoopMode};

pub fn delay<T>(inner: Anim<T>, by_frames: u64) -> Anim<T> {
    Anim::Expr(Expr::Delay {
        inner: Box::new(inner),
        by: by_frames,
    })
}

pub fn speed<T>(inner: Anim<T>, factor: f64) -> Anim<T> {
    Anim::Expr(Expr::Speed {
        inner: Box::new(inner),
        factor,
    })
}

pub fn reverse<T>(inner: Anim<T>, duration_frames: u64) -> Anim<T> {
    Anim::Expr(Expr::Reverse {
        inner: Box::new(inner),
        duration: duration_frames,
    })
}

pub fn loop_<T>(inner: Anim<T>, period_frames: u64, mode: LoopMode) -> Anim<T> {
    Anim::Expr(Expr::Loop {
        inner: Box::new(inner),
        period: period_frames,
        mode,
    })
}

pub fn mix<T>(a: Anim<T>, b: Anim<T>, t: Anim<f64>) -> Anim<T> {
    Anim::Expr(Expr::Mix {
        a: Box::new(a),
        b: Box::new(b),
        t: Box::new(t),
    })
}

pub fn sequence(a: Anim<f64>, a_len: u64, b: Anim<f64>) -> Anim<f64> {
    // Switch from `a` to `b` at `a_len`, with `b`'s time remapped so `b` starts at 0
    // when the switch occurs.
    let b_local = delay(b, a_len);
    let t_step = Anim::Keyframes(crate::anim::Keyframes {
        keys: vec![
            crate::anim::Keyframe {
                frame: crate::core::FrameIndex(0),
                value: 0.0,
                ease: crate::anim_ease::Ease::Linear,
            },
            crate::anim::Keyframe {
                frame: crate::core::FrameIndex(a_len),
                value: 1.0,
                ease: crate::anim_ease::Ease::Linear,
            },
        ],
        mode: crate::anim::InterpMode::Hold,
        default: None,
    });
    mix(a, b_local, t_step)
}

pub fn stagger(mut anims: Vec<(u64, Anim<f64>)>) -> Anim<f64> {
    anims.sort_by_key(|(offset, _)| *offset);
    let mut iter = anims.into_iter();
    let Some((first_offset, first_anim)) = iter.next() else {
        return Anim::constant(0.0);
    };

    let mut out = delay(first_anim, first_offset);
    for (offset, anim) in iter {
        out = sequence(out, offset, anim);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{InterpMode, Keyframe, Keyframes, SampleCtx};
    use crate::core::{Fps, FrameIndex};

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
                ease: crate::anim_ease::Ease::Linear,
            }],
            mode: InterpMode::Hold,
            default: None,
        });

        let s = sequence(a, 5, b);
        assert_eq!(s.sample(ctx(4)).unwrap(), 1.0);
        assert_eq!(s.sample(ctx(5)).unwrap(), 10.0);
    }
}
