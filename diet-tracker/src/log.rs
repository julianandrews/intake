use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogEntry {
    pub slug: String,
    pub hash: String,
    pub servings: f64,
    pub calories: u32,
    pub protein_g: f64,
    pub fiber_g: f64,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct LogFile {
    entries: Vec<LogEntry>,
}

#[derive(Debug)]
pub struct DayLog {
    pub date: NaiveDate,
    pub entries: Vec<LogEntry>,
}

fn log_path(log_dir: &Path, date: NaiveDate) -> PathBuf {
    log_dir.join(format!("{}.toml", date.format("%Y-%m-%d")))
}

pub fn append_entry(log_dir: &Path, date: NaiveDate, entry: &LogEntry) -> Result<()> {
    let path = log_path(log_dir, date);

    let mut log_file: LogFile = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read log: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("failed to parse log: {}", path.display()))?
    } else {
        LogFile { entries: Vec::new() }
    };

    log_file.entries.push(entry.clone());

    let content = toml::to_string(&log_file)
        .context("failed to serialize log")?;
    fs::write(&path, &content)
        .with_context(|| format!("failed to write log: {}", path.display()))?;

    Ok(())
}

pub fn list_log_dates(log_dir: &Path) -> Result<Vec<String>> {
    let mut dates = Vec::new();
    let entries = fs::read_dir(log_dir).context("failed to read log directory")?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "toml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                dates.push(stem.to_string());
            }
        }
    }
    dates.sort();
    Ok(dates)
}

pub fn load_day(log_dir: &Path, date: NaiveDate) -> Result<Option<DayLog>> {
    let path = log_path(log_dir, date);
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read log: {}", path.display()))?;

    let log_file: LogFile = toml::from_str(&content)
        .with_context(|| format!("failed to parse log: {}", path.display()))?;

    Ok(Some(DayLog { date, entries: log_file.entries }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_entry_roundtrip() -> Result<()> {
        let dir = std::env::temp_dir().join("diet-test-log");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let entry = LogEntry {
            slug: "oatmeal".to_string(),
            hash: "abc123".to_string(),
            servings: 1.5,
            calories: 200,
            protein_g: 15.0,
            fiber_g: 5.0,
            title: None,
        };

        append_entry(&dir, date, &entry)?;
        let loaded = load_day(&dir, date)?.expect("day log should exist");

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].slug, "oatmeal");
        assert_eq!(loaded.entries[0].hash, "abc123");
        assert_eq!(loaded.entries[0].servings, 1.5);
        assert_eq!(loaded.entries[0].calories, 200);
        assert!((loaded.entries[0].protein_g - 15.0).abs() < 0.001);
        assert!((loaded.entries[0].fiber_g - 5.0).abs() < 0.001);

        std::fs::remove_dir_all(&dir).context("cleanup failed")
    }

    #[test]
    fn test_log_entry_append_multiple() -> Result<()> {
        let dir = std::env::temp_dir().join("diet-test-log-multi");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 2).unwrap();

        append_entry(&dir, date, &LogEntry {
            slug: "coffee".to_string(),
            hash: "aaa".to_string(),
            servings: 2.0,
            calories: 12,
            protein_g: 0.0,
            fiber_g: 0.0,
            title: None,
        })?;

        append_entry(&dir, date, &LogEntry {
            slug: "oatmeal".to_string(),
            hash: "bbb".to_string(),
            servings: 1.0,
            calories: 418,
            protein_g: 22.0,
            fiber_g: 9.0,
            title: None,
        })?;

        let loaded = load_day(&dir, date)?.expect("day log should exist");
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].slug, "coffee");
        assert_eq!(loaded.entries[1].slug, "oatmeal");

        std::fs::remove_dir_all(&dir).context("cleanup failed")
    }

    #[test]
    fn test_load_nonexistent_day() -> Result<()> {
        let dir = std::env::temp_dir().join("diet-test-nonexistent");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let result = load_day(&dir, date)?;
        assert!(result.is_none());

        std::fs::remove_dir_all(&dir).context("cleanup failed")
    }

}
