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
#[path = "../../tests/unit/foundation/error.rs"]
mod tests;
