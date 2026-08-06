use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogEntry {
    pub slug: String,
    pub servings: f64,
    pub calories: u32,
    pub protein_g: f64,
    pub fiber_g: f64,
    pub fat_g: f64,
    pub carbs_g: f64,
    pub alcohol_g: f64,
    pub title: Option<String>,
}

impl LogEntry {
    pub fn total_calories(&self) -> f64 {
        self.calories as f64 * self.servings
    }

    pub fn total_protein(&self) -> f64 {
        self.protein_g * self.servings
    }

    pub fn total_fiber(&self) -> f64 {
        self.fiber_g * self.servings
    }

    pub fn total_fat(&self) -> f64 {
        self.fat_g * self.servings
    }

    pub fn total_carbs(&self) -> f64 {
        self.carbs_g * self.servings
    }

    pub fn total_alcohol(&self) -> f64 {
        self.alcohol_g * self.servings
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DayLog {
    pub entries: Vec<LogEntry>,
    #[serde(default)]
    pub exercise_calories: u32,
}

fn log_path(log_dir: &Path, date: NaiveDate) -> PathBuf {
    log_dir.join(format!("{}.toml", date.format("%Y-%m-%d")))
}

pub fn append_entry(log_dir: &Path, date: NaiveDate, entry: &LogEntry) -> Result<()> {
    let path = log_path(log_dir, date);

    let mut day_log: DayLog = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read log: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("failed to parse log: {}", path.display()))?
    } else {
        DayLog {
            entries: Vec::new(),
            exercise_calories: 0,
        }
    };

    day_log.entries.push(entry.clone());

    let content = toml::to_string(&day_log).context("failed to serialize log")?;
    fs::write(&path, &content)
        .with_context(|| format!("failed to write log: {}", path.display()))?;

    Ok(())
}

pub fn list_log_dates(log_dir: &Path) -> Result<Vec<String>> {
    let mut dates = Vec::new();
    let entries = fs::read_dir(log_dir)
        .with_context(|| format!("log directory not found: {}", log_dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
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

    let day_log: DayLog = toml::from_str(&content)
        .with_context(|| format!("failed to parse log: {}", path.display()))?;

    Ok(Some(day_log))
}

pub fn set_exercise_calories(log_dir: &Path, date: NaiveDate, calories: u32) -> Result<()> {
    let path = log_path(log_dir, date);

    let mut day_log: DayLog = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read log: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("failed to parse log: {}", path.display()))?
    } else {
        DayLog {
            entries: Vec::new(),
            exercise_calories: 0,
        }
    };

    day_log.exercise_calories = calories;

    let content = toml::to_string(&day_log).context("failed to serialize log")?;
    fs::write(&path, &content)
        .with_context(|| format!("failed to write log: {}", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_entry_roundtrip() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let entry = LogEntry {
            slug: "oatmeal".to_string(),
            servings: 1.5,
            calories: 200,
            protein_g: 15.0,
            fiber_g: 5.0,
            fat_g: 2.0,
            carbs_g: 30.0,
            alcohol_g: 0.0,
            title: None,
        };

        append_entry(dir.path(), date, &entry)?;
        let loaded = load_day(dir.path(), date)?.expect("day log should exist");

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].slug, "oatmeal");
        assert_eq!(loaded.entries[0].servings, 1.5);
        assert_eq!(loaded.entries[0].calories, 200);
        assert!((loaded.entries[0].protein_g - 15.0).abs() < 0.001);
        assert!((loaded.entries[0].fiber_g - 5.0).abs() < 0.001);
        assert!((loaded.entries[0].fat_g - 2.0).abs() < 0.001);
        assert!((loaded.entries[0].carbs_g - 30.0).abs() < 0.001);
        assert!((loaded.entries[0].alcohol_g - 0.0).abs() < 0.001);

        Ok(())
    }

    #[test]
    fn test_log_entry_old_format_missing_macros_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "[[entries]]\nslug = \"coffee\"\nservings = 1.0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\ntitle = \"Coffee\"\n",
        )?;

        assert!(load_day(dir.path(), date).is_err());

        Ok(())
    }

    #[test]
    fn test_totals_scale_by_servings() {
        let entry = LogEntry {
            slug: "test".to_string(),
            servings: 2.0,
            calories: 100,
            protein_g: 10.0,
            fiber_g: 5.0,
            fat_g: 4.0,
            carbs_g: 20.0,
            alcohol_g: 3.0,
            title: None,
        };
        assert_eq!(entry.total_calories(), 200.0);
        assert_eq!(entry.total_protein(), 20.0);
        assert_eq!(entry.total_fiber(), 10.0);
        assert_eq!(entry.total_fat(), 8.0);
        assert_eq!(entry.total_carbs(), 40.0);
        assert_eq!(entry.total_alcohol(), 6.0);
    }

    #[test]
    fn test_log_entry_append_multiple() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 2).unwrap();

        append_entry(
            dir.path(),
            date,
            &LogEntry {
                slug: "coffee".to_string(),
                servings: 2.0,
                calories: 12,
                protein_g: 0.0,
                fiber_g: 0.0,
                fat_g: 0.0,
                carbs_g: 0.0,
                alcohol_g: 0.0,
                title: None,
            },
        )?;

        append_entry(
            dir.path(),
            date,
            &LogEntry {
                slug: "oatmeal".to_string(),
                servings: 1.0,
                calories: 418,
                protein_g: 22.0,
                fiber_g: 9.0,
                fat_g: 6.0,
                carbs_g: 60.0,
                alcohol_g: 0.0,
                title: None,
            },
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day log should exist");
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].slug, "coffee");
        assert_eq!(loaded.entries[1].slug, "oatmeal");

        Ok(())
    }

    #[test]
    fn test_load_nonexistent_day() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let result = load_day(dir.path(), date)?;
        assert!(result.is_none());

        Ok(())
    }
}
