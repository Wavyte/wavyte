use super::*;

#[test]
fn fnv_seeded_hash_is_stable() {
    let mut a = Fnv1a64::new_default();
    a.write_bytes(b"wavyte");
    let mut b = Fnv1a64::new(Fnv1a64::OFFSET_BASIS);
    b.write_u8(b'w');
    b.write_bytes(b"avyte");
    assert_eq!(a.finish(), b.finish());
}

#[test]
fn mul_div255_variants_align() {
    for x in [0u16, 1, 127, 255] {
        for y in [0u16, 1, 127, 255] {
            assert_eq!(u16::from(mul_div255_u8(x, y)), mul_div255_u16(x, y));
        }
    }
}
