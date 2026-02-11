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
#[path = "../../tests/unit/foundation/math.rs"]
mod tests;
