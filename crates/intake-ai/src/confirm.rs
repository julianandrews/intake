use anyhow::Error as AnyhowError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmDecision {
    Accept,
    Reject,
    Feedback(String),
}

#[derive(Debug)]
pub enum ConfirmError {
    Cancelled,
    Io(AnyhowError),
}

impl From<AnyhowError> for ConfirmError {
    fn from(e: AnyhowError) -> Self {
        ConfirmError::Io(e)
    }
}

pub trait Confirmer {
    fn confirm(&mut self, rendered: &str) -> Result<ConfirmDecision, ConfirmError>;

    fn present_before_confirm(&self) -> bool {
        true
    }
}
