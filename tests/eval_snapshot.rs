use wavyte::{Composition, Evaluator, FrameIndex};

fn mix64(mut z: u64) -> u64 {
    // SplitMix64 mixing function.
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn digest_u64(bytes: &[u8]) -> u64 {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for chunk in bytes.chunks(8) {
        let mut v = 0u64;
        for (i, &b) in chunk.iter().enumerate() {
            v |= (b as u64) << (i * 8);
        }
        state = mix64(state ^ v);
    }
    state
}

#[test]
fn eval_snapshot_is_deterministic() {
    let s = include_str!("data/simple_comp.json");
    let comp: Composition = serde_json::from_str(s).unwrap();

    let mut digest = 0u64;
    for f in 0..20u64 {
        let g = Evaluator::eval_frame(&comp, FrameIndex(f)).unwrap();
        let bytes = serde_json::to_vec(&g).unwrap();
        digest ^= digest_u64(&bytes);
    }

    // Updated when semantics change (intentionally should be rare).
    let expected: u64 = 2_604_478_087_886_820_385;
    assert_eq!(digest, expected);
}
