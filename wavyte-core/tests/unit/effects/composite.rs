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
