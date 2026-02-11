use super::*;

#[test]
fn display_prefixes_are_stable() {
    assert!(
        WavyteError::validation("x")
            .to_string()
            .contains("validation error:")
    );
    assert!(
        WavyteError::animation("x")
            .to_string()
            .contains("animation error:")
    );
    assert!(
        WavyteError::evaluation("x")
            .to_string()
            .contains("evaluation error:")
    );
    assert!(
        WavyteError::serde("x")
            .to_string()
            .contains("serialization error:")
    );
}

#[test]
fn other_preserves_source() {
    let base = std::io::Error::other("boom");
    let err = WavyteError::Other(anyhow::Error::new(base));
    assert!(err.to_string().contains("boom"));
}
