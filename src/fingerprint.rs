use crate::{eval::EvaluatedGraph, model::BlendMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameFingerprint {
    pub hi: u64,
    pub lo: u64,
}

pub fn fingerprint_eval(eval: &EvaluatedGraph) -> FrameFingerprint {
    let mut a = Fnv1a64::new(0xcbf29ce484222325);
    let mut b = Fnv1a64::new(0x9ae16a3b2f90404f);

    write_u64_pair(&mut a, &mut b, eval.nodes.len() as u64);
    for node in &eval.nodes {
        write_str_pair(&mut a, &mut b, &node.clip_id);
        write_str_pair(&mut a, &mut b, &node.asset);
        write_i64_pair(&mut a, &mut b, i64::from(node.z));
        for c in node.transform.as_coeffs() {
            write_u64_pair(&mut a, &mut b, c.to_bits());
        }
        write_u64_pair(&mut a, &mut b, node.opacity.to_bits());
        write_u8_pair(
            &mut a,
            &mut b,
            match node.blend {
                BlendMode::Normal => 0,
            },
        );
        match node.source_time_s {
            Some(t) => {
                write_u8_pair(&mut a, &mut b, 1);
                write_u64_pair(&mut a, &mut b, t.to_bits());
            }
            None => write_u8_pair(&mut a, &mut b, 0),
        }

        write_u64_pair(&mut a, &mut b, node.effects.len() as u64);
        for fx in &node.effects {
            write_str_pair(&mut a, &mut b, &fx.kind);
            write_json_value_pair(&mut a, &mut b, &fx.params);
        }

        match &node.transition_in {
            Some(tr) => {
                write_u8_pair(&mut a, &mut b, 1);
                write_str_pair(&mut a, &mut b, &tr.kind);
                write_u64_pair(&mut a, &mut b, tr.progress.to_bits());
                write_json_value_pair(&mut a, &mut b, &tr.params);
            }
            None => write_u8_pair(&mut a, &mut b, 0),
        }
        match &node.transition_out {
            Some(tr) => {
                write_u8_pair(&mut a, &mut b, 1);
                write_str_pair(&mut a, &mut b, &tr.kind);
                write_u64_pair(&mut a, &mut b, tr.progress.to_bits());
                write_json_value_pair(&mut a, &mut b, &tr.params);
            }
            None => write_u8_pair(&mut a, &mut b, 0),
        }
    }

    FrameFingerprint {
        hi: a.finish(),
        lo: b.finish(),
    }
}

fn write_json_value_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: &serde_json::Value) {
    match v {
        serde_json::Value::Null => write_u8_pair(a, b, 0),
        serde_json::Value::Bool(x) => {
            write_u8_pair(a, b, 1);
            write_u8_pair(a, b, u8::from(*x));
        }
        serde_json::Value::Number(n) => {
            write_u8_pair(a, b, 2);
            write_str_pair(a, b, &n.to_string());
        }
        serde_json::Value::String(s) => {
            write_u8_pair(a, b, 3);
            write_str_pair(a, b, s);
        }
        serde_json::Value::Array(items) => {
            write_u8_pair(a, b, 4);
            write_u64_pair(a, b, items.len() as u64);
            for item in items {
                write_json_value_pair(a, b, item);
            }
        }
        serde_json::Value::Object(map) => {
            write_u8_pair(a, b, 5);
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            write_u64_pair(a, b, keys.len() as u64);
            for k in keys {
                write_str_pair(a, b, &k);
                write_json_value_pair(a, b, &map[&k]);
            }
        }
    }
}

fn write_u8_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: u8) {
    a.write_u8(v);
    b.write_u8(v);
}

fn write_u64_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: u64) {
    a.write_u64(v);
    b.write_u64(v);
}

fn write_i64_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: i64) {
    write_u64_pair(a, b, v as u64);
}

fn write_str_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, s: &str) {
    write_u64_pair(a, b, s.len() as u64);
    a.write_bytes(s.as_bytes());
    b.write_bytes(s.as_bytes());
}

#[derive(Clone, Copy)]
struct Fnv1a64(u64);

impl Fnv1a64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }

    fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        let mut h = self.0;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        self.0 = h;
    }

    fn finish(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Anim, BlendMode, Canvas, Clip, ClipProps, Composition, Evaluator, FrameIndex};

    fn comp_with_opacity(opacity: f64) -> Composition {
        Composition {
            fps: crate::Fps::new(30, 1).unwrap(),
            canvas: Canvas {
                width: 64,
                height: 64,
            },
            duration: FrameIndex(2),
            assets: std::collections::BTreeMap::from([(
                "p0".to_string(),
                crate::Asset::Path(crate::PathAsset {
                    svg_path_d: "M0,0 L10,0 L10,10 Z".to_string(),
                }),
            )]),
            tracks: vec![crate::Track {
                name: "main".to_string(),
                z_base: 0,
                layout_mode: crate::LayoutMode::Absolute,
                layout_gap_px: 0.0,
                layout_padding: crate::Edges::default(),
                layout_align_x: crate::LayoutAlignX::Start,
                layout_align_y: crate::LayoutAlignY::Start,
                layout_grid_columns: 2,
                clips: vec![Clip {
                    id: "c0".to_string(),
                    asset: "p0".to_string(),
                    range: crate::FrameRange::new(FrameIndex(0), FrameIndex(2)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(crate::Transform2D::default()),
                        opacity: Anim::constant(opacity),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![],
                    transition_in: None,
                    transition_out: None,
                }],
            }],
            seed: 1,
        }
    }

    #[test]
    fn fingerprint_is_deterministic_for_same_eval() {
        let comp = comp_with_opacity(1.0);
        let eval = Evaluator::eval_frame(&comp, FrameIndex(0)).unwrap();
        let a = fingerprint_eval(&eval);
        let b = fingerprint_eval(&eval);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_changes_when_scene_changes() {
        let a_comp = comp_with_opacity(1.0);
        let b_comp = comp_with_opacity(0.5);
        let a_eval = Evaluator::eval_frame(&a_comp, FrameIndex(0)).unwrap();
        let b_eval = Evaluator::eval_frame(&b_comp, FrameIndex(0)).unwrap();
        assert_ne!(fingerprint_eval(&a_eval), fingerprint_eval(&b_eval));
    }
}
