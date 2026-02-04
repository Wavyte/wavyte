use crate::error::WavyteResult;
use crate::transitions::WipeDir;

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

pub fn wipe_over_in_place(
    dst: &mut [u8],
    a: &[u8],
    b: &[u8],
    params: WipeParams,
) -> WavyteResult<()> {
    let WipeParams {
        width,
        height,
        t,
        dir,
        soft_edge,
    } = params;
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| crate::WavyteError::evaluation("wipe buffer size overflow"))?;

    if dst.len() != expected_len || a.len() != expected_len || b.len() != expected_len {
        return Err(crate::WavyteError::evaluation(
            "wipe_over_in_place expects buffers matching width*height*4",
        ));
    }

    let t = t.clamp(0.0, 1.0);
    let soft_edge = soft_edge.max(0.0);

    let axis_len = match dir {
        WipeDir::LeftToRight | WipeDir::RightToLeft => width as f32,
        WipeDir::TopToBottom | WipeDir::BottomToTop => height as f32,
    };
    let soft_px = soft_edge * axis_len;

    let edge = t * (axis_len + 2.0 * soft_px) - soft_px;
    let a_edge = edge - soft_px;
    let b_edge = edge + soft_px;

    for y in 0..height {
        for x in 0..width {
            let pos = match dir {
                WipeDir::LeftToRight => x as f32,
                WipeDir::RightToLeft => (width - 1 - x) as f32,
                WipeDir::TopToBottom => y as f32,
                WipeDir::BottomToTop => (height - 1 - y) as f32,
            };

            let m = if soft_px <= 0.0 {
                if pos < edge { 1.0 } else { 0.0 }
            } else {
                1.0 - smoothstep(a_edge, b_edge, pos)
            };

            let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
            let dp = [dst[idx], dst[idx + 1], dst[idx + 2], dst[idx + 3]];
            let ap = [a[idx], a[idx + 1], a[idx + 2], a[idx + 3]];
            let bp = [b[idx], b[idx + 1], b[idx + 2], b[idx + 3]];
            let blended = crossfade(ap, bp, m);
            let out = over(dp, blended, 1.0);
            dst[idx..idx + 4].copy_from_slice(&out);
        }
    }

    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct WipeParams {
    pub width: u32,
    pub height: u32,
    pub t: f32,
    pub dir: WipeDir,
    pub soft_edge: f32,
}

fn mul_div255(x: u16, y: u16) -> u8 {
    (((u32::from(x) * u32::from(y)) + 127) / 255) as u8
}

fn add_sat_u8(a: u8, b: u8) -> u8 {
    a.saturating_add(b)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    if x <= a {
        return 0.0;
    }
    if x >= b {
        return 1.0;
    }
    let t = (x - a) / (b - a);
    (t * t * (3.0 - 2.0 * t)).clamp(0.0, 1.0)
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

    #[test]
    fn wipe_ltr_endpoints_match_a_and_b() {
        let (w, h) = (4u32, 1u32);
        let a_px = [255u8, 0u8, 0u8, 255u8];
        let b_px = [0u8, 0u8, 255u8, 255u8];

        let a = a_px.repeat((w * h) as usize);
        let b = b_px.repeat((w * h) as usize);
        let mut dst = vec![0u8; (w * h * 4) as usize];

        wipe_over_in_place(
            &mut dst,
            &a,
            &b,
            WipeParams {
                width: w,
                height: h,
                t: 0.0,
                dir: WipeDir::LeftToRight,
                soft_edge: 0.0,
            },
        )
        .unwrap();
        assert_eq!(dst, a);

        dst.fill(0);
        wipe_over_in_place(
            &mut dst,
            &a,
            &b,
            WipeParams {
                width: w,
                height: h,
                t: 1.0,
                dir: WipeDir::LeftToRight,
                soft_edge: 0.0,
            },
        )
        .unwrap();
        assert_eq!(dst, b);
    }

    #[test]
    fn wipe_ltr_midpoint_splits_image() {
        let (w, h) = (4u32, 1u32);
        let a_px = [255u8, 0u8, 0u8, 255u8];
        let b_px = [0u8, 0u8, 255u8, 255u8];

        let a = a_px.repeat((w * h) as usize);
        let b = b_px.repeat((w * h) as usize);
        let mut dst = vec![0u8; (w * h * 4) as usize];

        wipe_over_in_place(
            &mut dst,
            &a,
            &b,
            WipeParams {
                width: w,
                height: h,
                t: 0.5,
                dir: WipeDir::LeftToRight,
                soft_edge: 0.0,
            },
        )
        .unwrap();
        assert_eq!(&dst[0..8], &b[0..8]); // first two pixels are B
        assert_eq!(&dst[8..16], &a[8..16]); // last two pixels are A
    }

    #[test]
    fn wipe_soft_edge_blends_near_boundary() {
        let (w, h) = (4u32, 1u32);
        let a_px = [255u8, 0u8, 0u8, 255u8];
        let b_px = [0u8, 0u8, 255u8, 255u8];

        let a = a_px.repeat((w * h) as usize);
        let b = b_px.repeat((w * h) as usize);
        let mut dst = vec![0u8; (w * h * 4) as usize];

        wipe_over_in_place(
            &mut dst,
            &a,
            &b,
            WipeParams {
                width: w,
                height: h,
                t: 0.5,
                dir: WipeDir::LeftToRight,
                soft_edge: 0.25,
            },
        )
        .unwrap();

        let mid = &dst[8..12]; // pixel 2
        assert!(mid[0] > 0 && mid[0] < 255);
        assert!(mid[2] > 0 && mid[2] < 255);
        assert_eq!(mid[3], 255);
    }

    #[test]
    fn wipe_negative_soft_edge_is_treated_as_zero() {
        let (w, h) = (4u32, 1u32);
        let a_px = [255u8, 0u8, 0u8, 255u8];
        let b_px = [0u8, 0u8, 255u8, 255u8];

        let a = a_px.repeat((w * h) as usize);
        let b = b_px.repeat((w * h) as usize);
        let mut dst = vec![0u8; (w * h * 4) as usize];

        wipe_over_in_place(
            &mut dst,
            &a,
            &b,
            WipeParams {
                width: w,
                height: h,
                t: 0.5,
                dir: WipeDir::LeftToRight,
                soft_edge: -1.0,
            },
        )
        .unwrap();
        assert_eq!(&dst[0..8], &b[0..8]);
        assert_eq!(&dst[8..16], &a[8..16]);
    }
}
