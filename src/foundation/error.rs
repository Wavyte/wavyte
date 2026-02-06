pub type WavyteResult<T> = Result<T, WavyteError>;

#[derive(thiserror::Error, Debug)]
pub enum WavyteError {
    #[error("validation error: {0}")]
    Validation(String),

    #[error("animation error: {0}")]
    Animation(String),

    #[error("evaluation error: {0}")]
    Evaluation(String),

    #[error("serialization error: {0}")]
    Serde(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl WavyteError {
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }

    pub fn animation(msg: impl Into<String>) -> Self {
        Self::Animation(msg.into())
    }

    pub fn evaluation(msg: impl Into<String>) -> Self {
        Self::Evaluation(msg.into())
    }

    pub fn serde(msg: impl Into<String>) -> Self {
        Self::Serde(msg.into())
    }
}

#[cfg(test)]
mod tests {
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
}
