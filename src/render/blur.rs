use crate::{WavyteError, WavyteResult};

pub fn blur_rgba8_premul(
    src: &[u8],
    width: u32,
    height: u32,
    radius: u32,
    sigma: f32,
) -> WavyteResult<Vec<u8>> {
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| WavyteError::evaluation("blur buffer size overflow"))?;
    if src.len() != expected_len {
        return Err(WavyteError::evaluation(
            "blur_rgba8_premul expects src matching width*height*4",
        ));
    }
    if radius == 0 {
        return Ok(src.to_vec());
    }

    let kernel = gaussian_kernel_q16(radius, sigma)?;
    let mut tmp = vec![0u8; expected_len];
    let mut out = vec![0u8; expected_len];

    horizontal_pass(src, &mut tmp, width, height, &kernel);
    vertical_pass(&tmp, &mut out, width, height, &kernel);
    Ok(out)
}

fn gaussian_kernel_q16(radius: u32, sigma: f32) -> WavyteResult<Vec<u32>> {
    if radius == 0 {
        return Ok(vec![1 << 16]);
    }
    if !sigma.is_finite() || sigma <= 0.0 {
        return Err(WavyteError::validation("blur sigma must be > 0"));
    }

    let r = radius as i32;
    let mut weights_f = Vec::<f64>::with_capacity((2 * r + 1) as usize);
    let mut sum = 0.0f64;
    let sigma = sigma as f64;
    let denom = 2.0 * sigma * sigma;
    for i in -r..=r {
        let x = i as f64;
        let w = (-x * x / denom).exp();
        weights_f.push(w);
        sum += w;
    }
    if sum <= 0.0 {
        return Err(WavyteError::evaluation("gaussian kernel sum is zero"));
    }

    let mut weights = Vec::<u32>::with_capacity(weights_f.len());
    let mut acc: i64 = 0;
    for &wf in &weights_f {
        let q = ((wf / sum) * 65536.0).round() as i64;
        let q = q.clamp(0, 65536);
        weights.push(q as u32);
        acc += q;
    }
    let target: i64 = 65536;
    let delta = target - acc;
    if delta != 0 {
        let mid = weights.len() / 2;
        let mid_val = i64::from(weights[mid]);
        let new_mid = (mid_val + delta).clamp(0, 65536);
        weights[mid] = new_mid as u32;
    }

    Ok(weights)
}

fn horizontal_pass(src: &[u8], dst: &mut [u8], width: u32, height: u32, k: &[u32]) {
    let radius = (k.len() / 2) as i32;
    let w = width as i32;
    for y in 0..height as i32 {
        for x in 0..w {
            let mut acc = [0u64; 4];
            for (ki, &kw) in k.iter().enumerate() {
                let dx = ki as i32 - radius;
                let sx = (x + dx).clamp(0, w - 1);
                let idx = ((y * w + sx) as usize) * 4;
                for c in 0..4 {
                    acc[c] += (kw as u64) * (src[idx + c] as u64);
                }
            }
            let out_idx = ((y * w + x) as usize) * 4;
            for c in 0..4 {
                dst[out_idx + c] = q16_to_u8(acc[c]);
            }
        }
    }
}

fn vertical_pass(src: &[u8], dst: &mut [u8], width: u32, height: u32, k: &[u32]) {
    let radius = (k.len() / 2) as i32;
    let w = width as i32;
    let h = height as i32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u64; 4];
            for (ki, &kw) in k.iter().enumerate() {
                let dy = ki as i32 - radius;
                let sy = (y + dy).clamp(0, h - 1);
                let idx = ((sy * w + x) as usize) * 4;
                for c in 0..4 {
                    acc[c] += (kw as u64) * (src[idx + c] as u64);
                }
            }
            let out_idx = ((y * w + x) as usize) * 4;
            for c in 0..4 {
                dst[out_idx + c] = q16_to_u8(acc[c]);
            }
        }
    }
}

fn q16_to_u8(acc: u64) -> u8 {
    let v = (acc + 32768) >> 16;
    (v.min(255)) as u8
}

#[cfg(test)]
mod tests {
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
}
