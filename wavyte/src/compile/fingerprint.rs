use crate::{
    composition::model::BlendMode, eval::evaluator::EvaluatedGraph, foundation::math::Fnv1a64,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// Deterministic 128-bit fingerprint of an evaluated frame graph.
pub struct FrameFingerprint {
    /// High 64 bits.
    pub hi: u64,
    /// Low 64 bits.
    pub lo: u64,
}

/// Compute a stable fingerprint for an evaluated frame.
///
/// Used by static-frame elision to skip rendering duplicate frame graphs.
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

#[cfg(test)]
#[path = "../../tests/unit/compile/fingerprint.rs"]
mod tests;
