use anyhow::{bail, Context, Result};
use std::io::Write;
use std::process::Command;

/// Resolve the editor command from `$VISUAL`, falling back to `$EDITOR`,
/// then `nano`. The editor value may include arguments (e.g.
/// `"code --wait"`); the temp file path is appended as the last argument.
fn resolve_editor(get: impl Fn(&str) -> Option<String>) -> (String, Vec<String>) {
    for key in ["VISUAL", "EDITOR"] {
        if let Some(value) = get(key) {
            let value = value.trim();
            if !value.is_empty() {
                let mut parts = value.split_whitespace();
                let program = parts.next().expect("non-empty editor value").to_string();
                return (program, parts.map(str::to_string).collect());
            }
        }
    }
    ("nano".to_string(), Vec::new())
}

/// Open `prefill` in the user's editor and return what they saved.
///
/// Returns `Ok(None)` when the file is left unchanged (an abort), `Ok(Some)`
/// with the edited content otherwise. Errors when no editor spawns, the
/// editor exits nonzero, or the temp file cannot be created or read.
pub(crate) fn capture_via_editor(prefill: &str, suffix: &str) -> Result<Option<String>> {
    let (program, mut args) = resolve_editor(|key| std::env::var(key).ok());

    let mut file = tempfile::Builder::new()
        .prefix("intake-edit-")
        .suffix(suffix)
        .tempfile()
        .context("failed to create a temp file for the editor")?;
    file.write_all(prefill.as_bytes())
        .context("failed to write the editor temp file")?;

    let path = file.path().to_path_buf();
    args.push(path.to_string_lossy().into_owned());

    let status = Command::new(&program)
        .args(&args)
        .status()
        .with_context(|| format!("failed to spawn editor '{program}'"))?;
    if !status.success() {
        bail!("editor '{program}' exited with {status}");
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read editor output from {}", path.display()))?;
    if content == prefill {
        Ok(None)
    } else {
        Ok(Some(content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_editor_prefers_visual() {
        let get = |key: &str| match key {
            "VISUAL" => Some("vim -u NONE".to_string()),
            "EDITOR" => Some("vi".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_editor(get),
            (
                "vim".to_string(),
                vec!["-u".to_string(), "NONE".to_string()]
            )
        );
    }

    #[test]
    fn test_resolve_editor_falls_back_to_editor() {
        let get = |key: &str| match key {
            "VISUAL" => None,
            "EDITOR" => Some("nano --backup".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_editor(get),
            ("nano".to_string(), vec!["--backup".to_string()])
        );
    }

    #[test]
    fn test_resolve_editor_defaults_to_nano() {
        assert_eq!(resolve_editor(|_| None), ("nano".to_string(), Vec::new()));
    }

    #[test]
    fn test_resolve_editor_ignores_blank_values() {
        let get = |key: &str| match key {
            "VISUAL" => Some("   ".to_string()),
            "EDITOR" => Some("vi".to_string()),
            _ => None,
        };
        assert_eq!(resolve_editor(get), ("vi".to_string(), Vec::new()));
    }
}
