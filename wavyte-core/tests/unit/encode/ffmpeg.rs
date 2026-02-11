use super::*;

#[test]
fn config_validation_catches_bad_values() {
    assert!(
        EncodeConfig {
            width: 0,
            height: 10,
            fps: 30,
            out_path: PathBuf::from("assets/out.mp4"),
            overwrite: true,
            audio: None,
        }
        .validate()
        .is_err()
    );

    assert!(
        EncodeConfig {
            width: 11,
            height: 10,
            fps: 30,
            out_path: PathBuf::from("assets/out.mp4"),
            overwrite: true,
            audio: None,
        }
        .validate()
        .is_err()
    );

    assert!(
        EncodeConfig {
            width: 10,
            height: 10,
            fps: 0,
            out_path: PathBuf::from("assets/out.mp4"),
            overwrite: true,
            audio: None,
        }
        .validate()
        .is_err()
    );
}

#[test]
fn flatten_premul_over_black_produces_expected_rgb() {
    // Premultiplied red @ 50% alpha => rgb is 128,0,0 when premul.
    let src = vec![128u8, 0u8, 0u8, 128u8];
    let mut dst = vec![0u8; 4];
    flatten_to_opaque_rgba8(&mut dst, &src, true, [0, 0, 0, 255]).unwrap();
    assert_eq!(dst, vec![128u8, 0u8, 0u8, 255u8]);
}

#[test]
fn flatten_straight_over_black_produces_expected_rgb() {
    // Straight red @ 50% alpha => rgb becomes 128,0,0 over black.
    let src = vec![255u8, 0u8, 0u8, 128u8];
    let mut dst = vec![0u8; 4];
    flatten_to_opaque_rgba8(&mut dst, &src, false, [0, 0, 0, 255]).unwrap();
    assert_eq!(dst, vec![128u8, 0u8, 0u8, 255u8]);
}
