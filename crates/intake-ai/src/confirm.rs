#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmDecision {
    Accept,
    Reject,
    Feedback(String),
}

/// Decides whether a resolved value is accepted.
///
/// The pipeline renders the resolved value and hands it to the confirmer,
/// which returns a [`ConfirmDecision`]: `Accept` finishes the resolve,
/// `Reject` aborts it, and `Feedback` supplies the user's comment and
/// re-runs the loop. A confirmer that cannot reach a decision returns an
/// error; the pipeline boxes it and passes it through unchanged.
pub trait Confirmer {
    fn confirm(
        &mut self,
        rendered: &str,
    ) -> Result<ConfirmDecision, Box<dyn std::error::Error + Send + Sync>>;
}
