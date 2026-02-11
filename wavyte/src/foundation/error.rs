/// Convenience result type used across Wavyte.
pub type WavyteResult<T> = Result<T, WavyteError>;

/// Top-level error taxonomy used by engine APIs.
#[derive(thiserror::Error, Debug)]
pub enum WavyteError {
    /// Invalid user-provided or composition data.
    #[error("validation error: {0}")]
    Validation(String),

    /// Errors while validating or sampling animation expressions.
    #[error("animation error: {0}")]
    Animation(String),

    /// Errors while evaluating timeline state for a frame.
    #[error("evaluation error: {0}")]
    Evaluation(String),

    /// Errors when serializing or deserializing data structures.
    #[error("serialization error: {0}")]
    Serde(String),

    /// Wrapped lower-level error from dependencies or IO.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl WavyteError {
    /// Build a [`WavyteError::Validation`] value.
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }

    /// Build a [`WavyteError::Animation`] value.
    pub fn animation(msg: impl Into<String>) -> Self {
        Self::Animation(msg.into())
    }

    /// Build a [`WavyteError::Evaluation`] value.
    pub fn evaluation(msg: impl Into<String>) -> Self {
        Self::Evaluation(msg.into())
    }

    /// Build a [`WavyteError::Serde`] value.
    pub fn serde(msg: impl Into<String>) -> Self {
        Self::Serde(msg.into())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/foundation/error.rs"]
mod tests;
