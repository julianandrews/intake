use crate::config::Config;
use crate::{food, log};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap_complete::{CompletionCandidate, Shell};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

fn completion_config() -> Option<&'static Config> {
    static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            Config::resolve(None, None).map(Some).unwrap_or_else(|e| {
                eprintln!("warning: failed to load config for completion: {e}");
                None
            })
        })
        .as_ref()
}

pub(crate) fn complete_foods() -> Vec<CompletionCandidate> {
    let config = match completion_config() {
        Some(c) => c,
        None => return Vec::new(),
    };
    let dir = config.foods_dir();
    if !dir.is_dir() {
        return Vec::new();
    }
    match food::list_food_names(&dir) {
        Ok(names) => names.into_iter().map(CompletionCandidate::new).collect(),
        Err(e) => {
            eprintln!("warning: failed to list foods for completion: {e}");
            Vec::new()
        }
    }
}

pub(crate) fn complete_log_dates() -> Vec<CompletionCandidate> {
    let config = match completion_config() {
        Some(c) => c,
        None => return Vec::new(),
    };
    let dir = config.log_dir();
    if !dir.is_dir() {
        return Vec::new();
    }
    match log::list_log_dates(&dir) {
        Ok(dates) => dates
            .into_iter()
            .filter(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok())
            .map(CompletionCandidate::new)
            .collect(),
        Err(e) => {
            eprintln!("warning: failed to list log dates for completion: {e}");
            Vec::new()
        }
    }
}

fn completion_path(shell: &Shell) -> Result<PathBuf> {
    let (dir, filename) = match shell {
        Shell::Bash => (
            dirs::data_dir()
                .context("no data directory found")?
                .join("bash-completion")
                .join("completions"),
            "intake".to_string(),
        ),
        Shell::Zsh => (
            dirs::data_dir()
                .context("no data directory found")?
                .join("zsh")
                .join("completions"),
            "_intake".to_string(),
        ),
        Shell::Fish => (
            dirs::config_dir()
                .context("no config directory found")?
                .join("fish")
                .join("completions"),
            "intake.fish".to_string(),
        ),
        _ => anyhow::bail!("install not supported for {} shell", shell),
    };
    Ok(dir.join(filename))
}

pub(crate) fn cmd_completions(
    writer: &mut impl Write,
    shell: Shell,
    install: bool,
    command: clap::Command,
) -> Result<()> {
    if install {
        let path = completion_path(&shell)?;
        fs::create_dir_all(path.parent().context("completion path has no parent")?)?;
        let completer = std::env::current_exe().context("failed to resolve current executable")?;
        let output = std::process::Command::new(completer)
            .env("COMPLETE", shell.to_string())
            .output()
            .context("failed to generate completion script")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("completion generation failed: {stderr}");
        }
        fs::write(&path, &output.stdout)
            .with_context(|| format!("failed to write {}", path.display()))?;
        writeln!(
            writer,
            "Installed {} completions to {}",
            shell,
            path.display()
        )?;
    } else {
        let mut cmd = command;
        clap_complete::generate(shell, &mut cmd, "intake", &mut *writer);
    }
    Ok(())
}
