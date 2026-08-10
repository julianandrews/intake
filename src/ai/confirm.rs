use intake_ai::confirm::{ConfirmDecision, ConfirmError, Confirmer};
use std::io::{BufRead, Write};

/// The proposal text `cmd_ai_log`'s `present` emits when the applied ops
/// leave the day unchanged. It goes through the normal present-and-confirm
/// flow: the user accepts (nothing is written), rejects, or gives feedback.
pub(crate) const NO_CHANGES_PROPOSAL: &str = "No changes.";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AiAnswer {
    Yes,
    No,
    Feedback,
}

fn classify_ai(line: &str) -> Option<AiAnswer> {
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Some(AiAnswer::Yes),
        "n" | "no" => Some(AiAnswer::No),
        "f" | "feedback" => Some(AiAnswer::Feedback),
        _ => None,
    }
}

/// The three-way AI confirmer: prints the rendered proposal to stdout, then
/// prompts `[y]es` / `[n]o` / `[f]eedback` on stderr. EOF on either prompt
/// is a cancellation. Feedback reads an inline instruction and re-runs
/// generation with the conversation continued.
pub(crate) struct AiConfirmer<'a> {
    writer: &'a mut dyn Write,
}

impl<'a> AiConfirmer<'a> {
    pub(crate) fn new(writer: &'a mut dyn Write) -> AiConfirmer<'a> {
        AiConfirmer { writer }
    }
}

fn io_confirm_err(e: std::io::Error) -> ConfirmError {
    ConfirmError::Io(e.into())
}

impl Confirmer for AiConfirmer<'_> {
    fn confirm(&mut self, rendered: &str) -> Result<ConfirmDecision, ConfirmError> {
        self.writer
            .write_all(rendered.as_bytes())
            .map_err(io_confirm_err)?;
        if !rendered.ends_with('\n') {
            self.writer.write_all(b"\n").map_err(io_confirm_err)?;
        }
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            eprint!("[y]es / [n]o / [f]eedback: ");
            std::io::stderr().flush().ok();
            line.clear();
            if stdin.lock().read_line(&mut line).map_err(io_confirm_err)? == 0 {
                return Err(ConfirmError::Cancelled);
            }
            match classify_ai(&line) {
                Some(AiAnswer::Yes) => return Ok(ConfirmDecision::Accept),
                Some(AiAnswer::No) => return Ok(ConfirmDecision::Reject),
                Some(AiAnswer::Feedback) => {
                    eprint!("Feedback: ");
                    std::io::stderr().flush().ok();
                    line.clear();
                    if stdin.lock().read_line(&mut line).map_err(io_confirm_err)? == 0 {
                        return Err(ConfirmError::Cancelled);
                    }
                    let msg = line.trim();
                    if msg.is_empty() {
                        continue;
                    }
                    return Ok(ConfirmDecision::Feedback(msg.to_string()));
                }
                None => {}
            }
        }
    }
}

/// Accepts the proposal without rendering or prompting, for `--yes`.
pub(crate) struct ConfirmAlways;

impl Confirmer for ConfirmAlways {
    fn confirm(&mut self, _rendered: &str) -> Result<ConfirmDecision, ConfirmError> {
        Ok(ConfirmDecision::Accept)
    }

    fn present_before_confirm(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_ai_answers() {
        for line in ["y", "Y", "yes", "YES\n", " yes "] {
            assert!(
                matches!(classify_ai(line), Some(AiAnswer::Yes)),
                "line: {line:?}"
            );
        }
        for line in ["n", "N", "no", "nO\n"] {
            assert!(
                matches!(classify_ai(line), Some(AiAnswer::No)),
                "line: {line:?}"
            );
        }
        for line in ["f", "F", "feedback", "feedback\n"] {
            assert!(
                matches!(classify_ai(line), Some(AiAnswer::Feedback)),
                "line: {line:?}"
            );
        }
        for line in ["", "maybe", "yess", "x"] {
            assert_eq!(classify_ai(line), None, "line: {line:?}");
        }
    }

    #[test]
    fn test_confirm_always_skips_present() {
        let mut always = ConfirmAlways;
        assert!(!always.present_before_confirm());
        assert!(matches!(
            always.confirm("anything").unwrap(),
            ConfirmDecision::Accept
        ));
    }
}
