#[derive(Clone, Copy, Debug)]
pub(crate) struct Fnv1a64(u64);

impl Fnv1a64 {
    pub(crate) const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01B3;

    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn new_default() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    pub(crate) fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }

    pub(crate) fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        let mut h = self.0;
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(Self::PRIME);
        }
        self.0 = h;
    }

    pub(crate) fn finish(self) -> u64 {
        self.0
    }
}

pub(crate) fn mul_div255_u16(x: u16, y: u16) -> u16 {
    (((u32::from(x) * u32::from(y)) + 127) / 255) as u16
}

pub(crate) fn mul_div255_u8(x: u16, y: u16) -> u8 {
    mul_div255_u16(x, y) as u8
}

#[cfg(test)]
mod tests {
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
}
