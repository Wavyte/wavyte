use super::*;

#[test]
fn blur_radius_0_is_identity() {
    let src = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
    let out = blur_rgba8_premul(&src, 1, 2, 0, 1.0).unwrap();
    assert_eq!(out, src);
}

#[test]
fn blur_constant_image_is_identity() {
    let (w, h) = (4u32, 3u32);
    let px = [10u8, 20u8, 30u8, 40u8];
    let src = px.repeat((w * h) as usize);
    let out = blur_rgba8_premul(&src, w, h, 3, 2.0).unwrap();
    assert_eq!(out, src);
}

#[test]
fn blur_spreads_energy_from_single_pixel() {
    let (w, h) = (5u32, 5u32);
    let mut src = vec![0u8; (w * h * 4) as usize];
    let center = ((2 * w + 2) * 4) as usize;
    src[center..center + 4].copy_from_slice(&[255, 255, 255, 255]);

    let out = blur_rgba8_premul(&src, w, h, 2, 1.2).unwrap();

    let nonzero = out.chunks_exact(4).filter(|px| px[3] != 0).count();
    assert!(nonzero > 1);

    let sum_a: u32 = out.chunks_exact(4).map(|px| u32::from(px[3])).sum();
    assert!((sum_a as i32 - 255).abs() <= 4);
}
