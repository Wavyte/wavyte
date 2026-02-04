use crate::error::WavyteResult;

pub type PremulRgba8 = [u8; 4];

pub fn over(dst: PremulRgba8, src: PremulRgba8, opacity: f32) -> PremulRgba8 {
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 || src[3] == 0 {
        return dst;
    }

    let op = ((opacity * 255.0).round() as i32).clamp(0, 255) as u16;
    let sa = mul_div255(u16::from(src[3]), op);
    if sa == 0 {
        return dst;
    }

    let inv = 255u16 - u16::from(sa);

    let mut out = [0u8; 4];
    out[3] = add_sat_u8(sa, mul_div255(u16::from(dst[3]), inv));

    for i in 0..3 {
        let sc = mul_div255(u16::from(src[i]), op);
        let dc = mul_div255(u16::from(dst[i]), inv);
        out[i] = add_sat_u8(sc, dc);
    }
    out
}

pub fn crossfade(a: PremulRgba8, b: PremulRgba8, t: f32) -> PremulRgba8 {
    let t = t.clamp(0.0, 1.0);
    let tt = ((t * 255.0).round() as i32).clamp(0, 255) as u16;
    let it = 255u16 - tt;

    let mut out = [0u8; 4];
    for i in 0..4 {
        let av = mul_div255(u16::from(a[i]), it);
        let bv = mul_div255(u16::from(b[i]), tt);
        out[i] = add_sat_u8(av, bv);
    }
    out
}

pub fn over_in_place(dst: &mut [u8], src: &[u8], opacity: f32) -> WavyteResult<()> {
    if dst.len() != src.len() || !dst.len().is_multiple_of(4) {
        return Err(crate::WavyteError::evaluation(
            "over_in_place expects equal-length rgba8 buffers",
        ));
    }
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let out = over([d[0], d[1], d[2], d[3]], [s[0], s[1], s[2], s[3]], opacity);
        d.copy_from_slice(&out);
    }
    Ok(())
}

pub fn crossfade_over_in_place(dst: &mut [u8], a: &[u8], b: &[u8], t: f32) -> WavyteResult<()> {
    if dst.len() != a.len() || dst.len() != b.len() || !dst.len().is_multiple_of(4) {
        return Err(crate::WavyteError::evaluation(
            "crossfade_over_in_place expects equal-length rgba8 buffers",
        ));
    }
    for ((d, a), b) in dst
        .chunks_exact_mut(4)
        .zip(a.chunks_exact(4))
        .zip(b.chunks_exact(4))
    {
        let blended = crossfade([a[0], a[1], a[2], a[3]], [b[0], b[1], b[2], b[3]], t);
        let out = over([d[0], d[1], d[2], d[3]], blended, 1.0);
        d.copy_from_slice(&out);
    }
    Ok(())
}

fn mul_div255(x: u16, y: u16) -> u8 {
    (((u32::from(x) * u32::from(y)) + 127) / 255) as u8
}

fn add_sat_u8(a: u8, b: u8) -> u8 {
    a.saturating_add(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_opacity_0_is_noop() {
        let dst = [1, 2, 3, 4];
        let src = [200, 200, 200, 200];
        assert_eq!(over(dst, src, 0.0), dst);
    }

    #[test]
    fn over_src_alpha_0_is_noop() {
        let dst = [10, 20, 30, 40];
        let src = [255, 255, 255, 0];
        assert_eq!(over(dst, src, 1.0), dst);
    }

    #[test]
    fn over_src_opaque_replaces_dst() {
        let dst = [0, 0, 0, 255];
        let src = [255, 0, 0, 255];
        assert_eq!(over(dst, src, 1.0), src);
    }

    #[test]
    fn over_dst_transparent_returns_scaled_src() {
        let dst = [0, 0, 0, 0];
        let src = [100, 110, 120, 200];
        assert_eq!(over(dst, src, 1.0), src);
    }

    #[test]
    fn crossfade_t_0_is_a_and_t_1_is_b() {
        let a = [10, 20, 30, 40];
        let b = [200, 210, 220, 230];
        assert_eq!(crossfade(a, b, 0.0), a);
        assert_eq!(crossfade(a, b, 1.0), b);
    }
}
