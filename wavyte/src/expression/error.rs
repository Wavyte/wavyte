use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExprError {
    pub(crate) offset: usize,
    pub(crate) message: String,
}

impl ExprError {
    pub(crate) fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expr error at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for ExprError {}
