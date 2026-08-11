use intake_ai::confirm::{ConfirmDecision, Confirmer};
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
/// is a decline: the confirmer notes it on stderr and rejects, so the
/// command treats it like any other non-affirmative answer. Feedback reads
/// an inline instruction and re-runs generation with the conversation
/// continued.
pub(crate) struct AiConfirmer<'a> {
    writer: &'a mut dyn Write,
}

impl<'a> AiConfirmer<'a> {
    pub(crate) fn new(writer: &'a mut dyn Write) -> AiConfirmer<'a> {
        AiConfirmer { writer }
    }
}

fn io_confirm_err(e: std::io::Error) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(e)
}

/// Closed stdin mid-prompt: warn on stderr that nothing was done, then
/// decline so the command exits 0 like any other non-affirmative answer.
fn stdin_closed() -> Result<ConfirmDecision, Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("no confirmation received — nothing written; use `--yes` to skip confirmation");
    Ok(ConfirmDecision::Reject)
}

impl Confirmer for AiConfirmer<'_> {
    fn confirm(
        &mut self,
        rendered: &str,
    ) -> Result<ConfirmDecision, Box<dyn std::error::Error + Send + Sync>> {
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
                return stdin_closed();
            }
            match classify_ai(&line) {
                Some(AiAnswer::Yes) => return Ok(ConfirmDecision::Accept),
                Some(AiAnswer::No) => return Ok(ConfirmDecision::Reject),
                Some(AiAnswer::Feedback) => {
                    eprint!("Feedback: ");
                    std::io::stderr().flush().ok();
                    line.clear();
                    if stdin.lock().read_line(&mut line).map_err(io_confirm_err)? == 0 {
                        return stdin_closed();
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
}
