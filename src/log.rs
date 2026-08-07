use crate::amount::{Grams, Servings};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogEntry {
    pub slug: String,
    pub servings: Servings,
    pub calories: u32,
    pub protein_g: Grams,
    pub fiber_g: Grams,
    pub fat_g: Grams,
    pub carbs_g: Grams,
    pub alcohol_g: Grams,
    pub title: Option<String>,
}

impl LogEntry {
    pub fn total_calories(&self) -> Decimal {
        Decimal::from(self.calories) * self.servings.to_decimal()
    }

    pub fn total_protein(&self) -> Grams {
        self.protein_g * self.servings.to_decimal()
    }

    pub fn total_fiber(&self) -> Grams {
        self.fiber_g * self.servings.to_decimal()
    }

    pub fn total_fat(&self) -> Grams {
        self.fat_g * self.servings.to_decimal()
    }

    pub fn total_carbs(&self) -> Grams {
        self.carbs_g * self.servings.to_decimal()
    }

    pub fn total_alcohol(&self) -> Grams {
        self.alcohol_g * self.servings.to_decimal()
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

    fn entry(
        slug: &str,
        servings: f64,
        calories: u32,
        protein: f64,
        fiber: f64,
        fat: f64,
        carbs: f64,
        alcohol: f64,
    ) -> LogEntry {
        LogEntry {
            slug: slug.to_string(),
            servings: Servings::from_f64(servings).unwrap(),
            calories,
            protein_g: Grams::from_f64(protein).unwrap(),
            fiber_g: Grams::from_f64(fiber).unwrap(),
            fat_g: Grams::from_f64(fat).unwrap(),
            carbs_g: Grams::from_f64(carbs).unwrap(),
            alcohol_g: Grams::from_f64(alcohol).unwrap(),
            title: None,
        }
    }

    #[test]
    fn test_log_entry_roundtrip() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let e = entry("oatmeal", 1.5, 200, 15.0, 5.0, 2.0, 30.0, 0.0);

        append_entry(dir.path(), date, &e)?;
        let loaded = load_day(dir.path(), date)?.expect("day log should exist");

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].slug, "oatmeal");
        assert_eq!(loaded.entries[0].servings, Servings::from_f64(1.5).unwrap());
        assert_eq!(loaded.entries[0].calories, 200);
        assert_eq!(loaded.entries[0].protein_g, Grams::from_f64(15.0).unwrap());
        assert_eq!(loaded.entries[0].fiber_g, Grams::from_f64(5.0).unwrap());
        assert_eq!(loaded.entries[0].fat_g, Grams::from_f64(2.0).unwrap());
        assert_eq!(loaded.entries[0].carbs_g, Grams::from_f64(30.0).unwrap());
        assert_eq!(loaded.entries[0].alcohol_g, Grams::from_f64(0.0).unwrap());

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
    fn test_log_entry_non_positive_servings_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "[[entries]]\nslug = \"coffee\"\nservings = 0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());

        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "[[entries]]\nslug = \"coffee\"\nservings = -2.0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());

        Ok(())
    }

    #[test]
    fn test_log_entry_negative_macros_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "[[entries]]\nslug = \"coffee\"\nservings = 1.0\ncalories = 12\nprotein_g = -1\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());

        Ok(())
    }

    #[test]
    fn test_log_entry_legacy_float_values_normalized() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "[[entries]]\nslug = \"chili\"\nservings = 1.0\ncalories = 300\nprotein_g = 3.3333333333333335\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Chili\"\n",
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day log should exist");
        assert_eq!(loaded.entries[0].protein_g, Grams::from_f64(3.333).unwrap());

        Ok(())
    }

    #[test]
    fn test_totals_scale_by_servings() {
        let e = entry("test", 2.0, 100, 10.0, 5.0, 4.0, 20.0, 3.0);
        assert_eq!(e.total_calories(), Decimal::from(200));
        assert_eq!(e.total_protein(), Grams::from_f64(20.0).unwrap());
        assert_eq!(e.total_fiber(), Grams::from_f64(10.0).unwrap());
        assert_eq!(e.total_fat(), Grams::from_f64(8.0).unwrap());
        assert_eq!(e.total_carbs(), Grams::from_f64(40.0).unwrap());
        assert_eq!(e.total_alcohol(), Grams::from_f64(6.0).unwrap());
    }

    #[test]
    fn test_totals_fractional_servings() {
        let e = entry("test", 1.5, 100, 10.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(e.total_calories(), Decimal::from(150));
        assert_eq!(e.total_protein(), Grams::from_f64(15.0).unwrap());
    }

    #[test]
    fn test_log_entry_append_multiple() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 2).unwrap();

        append_entry(
            dir.path(),
            date,
            &entry("coffee", 2.0, 12, 0.0, 0.0, 0.0, 0.0, 0.0),
        )?;

        append_entry(
            dir.path(),
            date,
            &entry("oatmeal", 1.0, 418, 22.0, 9.0, 6.0, 60.0, 0.0),
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
