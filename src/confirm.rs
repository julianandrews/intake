use anyhow::Result;
use std::io::{BufRead, Write};

/// Map an answer line onto a yes/no decision; `None` for anything else.
fn classify(line: &str) -> Option<bool> {
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

/// Report a canceled confirmation (stdin closed): warn on stderr that
/// nothing was done, and print `Nothing {verb}` to `writer`.
pub(crate) fn nothing_confirmed(writer: &mut impl Write, verb: &str) -> Result<()> {
    eprintln!("no confirmation received — nothing {verb}; use `--yes` to skip confirmation");
    writeln!(writer, "Nothing {verb}")?;
    Ok(())
}

/// Prompt `[y]es` / `[n]o` on stderr and read the answer from stdin.
///
/// Returns `Some(true)` on yes, `Some(false)` on no, and `None` when stdin
/// is closed (EOF) — a cancellation, not an error. Non-answers reprompt.
pub(crate) fn confirm_yes_no(prompt: &str) -> Result<Option<bool>> {
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        eprint!("{prompt} [y]es / [n]o: ");
        std::io::stderr().flush().ok();
        line.clear();
        if stdin.lock().read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if let Some(answer) = classify(&line) {
            return Ok(Some(answer));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_yes() {
        for line in ["y", "Y", "yes", "YES", " yes ", "y\n", "Yes\n"] {
            assert_eq!(classify(line), Some(true), "line: {line:?}");
        }
    }

    #[test]
    fn test_classify_no() {
        for line in ["n", "N", "no", "nO", " no\n"] {
            assert_eq!(classify(line), Some(false), "line: {line:?}");
        }
    }

    #[test]
    fn test_classify_other() {
        for line in ["", " ", "maybe", "yn", "yess"] {
            assert_eq!(classify(line), None, "line: {line:?}");
        }
    }
}
