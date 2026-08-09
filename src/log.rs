use crate::amount::{Calories, Grams, Macros, Servings};
use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogEntry {
    pub title: String,
    pub servings: Servings,
    pub calories: Calories,
    pub protein_g: Grams,
    pub fiber_g: Grams,
    pub fat_g: Grams,
    pub carbs_g: Grams,
    pub alcohol_g: Grams,
}

impl LogEntry {
    pub fn total_calories(&self) -> Result<Calories> {
        self.calories
            .checked_mul(self.servings.to_decimal())
            .ok_or_else(|| anyhow!("calorie total overflow for '{}'", self.title))
    }

    pub fn total_protein(&self) -> Result<Grams> {
        self.protein_g
            .checked_mul(self.servings.to_decimal())
            .ok_or_else(|| anyhow!("protein total overflow for '{}'", self.title))
    }

    pub fn total_fiber(&self) -> Result<Grams> {
        self.fiber_g
            .checked_mul(self.servings.to_decimal())
            .ok_or_else(|| anyhow!("fiber total overflow for '{}'", self.title))
    }

    pub fn total_fat(&self) -> Result<Grams> {
        self.fat_g
            .checked_mul(self.servings.to_decimal())
            .ok_or_else(|| anyhow!("fat total overflow for '{}'", self.title))
    }

    pub fn total_carbs(&self) -> Result<Grams> {
        self.carbs_g
            .checked_mul(self.servings.to_decimal())
            .ok_or_else(|| anyhow!("carbs total overflow for '{}'", self.title))
    }

    pub fn total_alcohol(&self) -> Result<Grams> {
        self.alcohol_g
            .checked_mul(self.servings.to_decimal())
            .ok_or_else(|| anyhow!("alcohol total overflow for '{}'", self.title))
    }

    /// All macro totals scaled by servings; errors on overflow.
    pub fn totals(&self) -> Result<Macros> {
        Ok(Macros {
            calories: self.total_calories()?,
            protein_g: self.total_protein()?,
            fiber_g: self.total_fiber()?,
            fat_g: self.total_fat()?,
            carbs_g: self.total_carbs()?,
            alcohol_g: self.total_alcohol()?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DayLog {
    pub entries: Vec<LogEntry>,
    pub exercise_calories: Calories,
}

fn log_path(log_dir: &Path, date: NaiveDate) -> PathBuf {
    log_dir.join(format!("{}.toml", date.format("%Y-%m-%d")))
}

fn update_day<F>(log_dir: &Path, date: NaiveDate, mutate: F) -> Result<()>
where
    F: FnOnce(&mut DayLog),
{
    let path = log_path(log_dir, date);

    let mut day_log: DayLog = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read log: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("failed to parse log: {}", path.display()))?
    } else {
        DayLog {
            entries: Vec::new(),
            exercise_calories: Calories::ZERO,
        }
    };

    mutate(&mut day_log);

    let content = toml::to_string(&day_log).context("failed to serialize log")?;
    fs::create_dir_all(log_dir)
        .with_context(|| format!("failed to create log directory: {}", log_dir.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(log_dir)
        .with_context(|| format!("failed to create temporary log in: {}", log_dir.display()))?;
    tmp.write_all(content.as_bytes())
        .with_context(|| format!("failed to write temporary log: {}", path.display()))?;
    tmp.persist(&path)
        .with_context(|| format!("failed to write log: {}", path.display()))?;

    Ok(())
}

pub fn append_entry(log_dir: &Path, date: NaiveDate, entry: &LogEntry) -> Result<()> {
    update_day(log_dir, date, |day| day.entries.push(entry.clone()))
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

pub fn set_exercise_calories(log_dir: &Path, date: NaiveDate, calories: Calories) -> Result<()> {
    update_day(log_dir, date, |day| day.exercise_calories = calories)
}

pub(crate) fn day_net_and_deficit(
    calories: Decimal,
    exercise_calories: Calories,
    maintenance_calories: Option<Calories>,
) -> Result<(Decimal, Option<Decimal>)> {
    let exercise = exercise_calories.to_decimal();
    let net_cal = calories
        .checked_sub(exercise)
        .context("net calorie total overflow")?;
    let deficit = maintenance_calories
        .map(|mc| {
            mc.to_decimal()
                .checked_sub(net_cal)
                .context("deficit overflow")
        })
        .transpose()?;
    Ok((net_cal, deficit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn entry(
        title: &str,
        servings: &str,
        calories: u32,
        protein: &str,
        fiber: &str,
        fat: &str,
        carbs: &str,
        alcohol: &str,
    ) -> LogEntry {
        LogEntry {
            title: title.to_string(),
            servings: Servings::from_str(servings).unwrap(),
            calories: Calories::from_u32(calories),
            protein_g: Grams::from_str(protein).unwrap(),
            fiber_g: Grams::from_str(fiber).unwrap(),
            fat_g: Grams::from_str(fat).unwrap(),
            carbs_g: Grams::from_str(carbs).unwrap(),
            alcohol_g: Grams::from_str(alcohol).unwrap(),
        }
    }

    #[test]
    fn test_log_entry_roundtrip() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let e = entry("oatmeal", "1.5", 200, "15.0", "5.0", "2.0", "30.0", "0.0");

        append_entry(dir.path(), date, &e)?;
        let loaded = load_day(dir.path(), date)?.expect("day log should exist");

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].title, "oatmeal");
        assert_eq!(
            loaded.entries[0].servings,
            Servings::from_str("1.5").unwrap()
        );
        assert_eq!(loaded.entries[0].calories, Calories::from_u32(200));
        assert_eq!(
            loaded.entries[0].protein_g,
            Grams::from_str("15.0").unwrap()
        );
        assert_eq!(loaded.entries[0].fiber_g, Grams::from_str("5.0").unwrap());
        assert_eq!(loaded.entries[0].fat_g, Grams::from_str("2.0").unwrap());
        assert_eq!(loaded.entries[0].carbs_g, Grams::from_str("30.0").unwrap());
        assert_eq!(loaded.entries[0].alcohol_g, Grams::from_str("0.0").unwrap());

        Ok(())
    }

    #[test]
    fn test_log_entry_old_format_missing_macros_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "[[entries]]\nservings = 1.0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\ntitle = \"Coffee\"\n",
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
            "[[entries]]\nservings = 0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());

        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "[[entries]]\nservings = -2.0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\n",
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
            "[[entries]]\nservings = 1.0\ncalories = 12\nprotein_g = -1\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());

        Ok(())
    }

    #[test]
    fn test_log_entry_negative_calories_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "[[entries]]\nservings = 1.0\ncalories = -12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());

        Ok(())
    }

    #[test]
    fn test_log_entry_float_literals_normalized() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "exercise_calories = 0\n\n[[entries]]\nservings = 1.0\ncalories = 300\nprotein_g = 3.3333333333333335\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Chili\"\n",
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day log should exist");
        assert_eq!(
            loaded.entries[0].protein_g,
            Grams::from_str("3.333").unwrap()
        );

        Ok(())
    }

    #[test]
    fn test_totals_scale_by_servings() {
        let e = entry("test", "2.0", 100, "10.0", "5.0", "4.0", "20.0", "3.0");
        assert_eq!(e.total_calories().unwrap(), Calories::from_u32(200));
        assert_eq!(e.total_protein().unwrap(), Grams::from_str("20.0").unwrap());
        assert_eq!(e.total_fiber().unwrap(), Grams::from_str("10.0").unwrap());
        assert_eq!(e.total_fat().unwrap(), Grams::from_str("8.0").unwrap());
        assert_eq!(e.total_carbs().unwrap(), Grams::from_str("40.0").unwrap());
        assert_eq!(e.total_alcohol().unwrap(), Grams::from_str("6.0").unwrap());
    }

    #[test]
    fn test_totals_fractional_servings() {
        let e = entry("test", "1.5", 100, "10.0", "0.0", "0.0", "0.0", "0.0");
        assert_eq!(e.total_calories().unwrap(), Calories::from_u32(150));
        assert_eq!(e.total_protein().unwrap(), Grams::from_str("15.0").unwrap());
    }

    #[test]
    fn test_totals_fractional_calories() {
        let mut e = entry("test", "3.0", 0, "0.0", "0.0", "0.0", "0.0", "0.0");
        e.calories = Calories::from_str("33.333").unwrap();
        assert_eq!(
            e.total_calories().unwrap(),
            Calories::from_str("99.999").unwrap()
        );
    }

    #[test]
    fn test_totals_overflow_errors() {
        let mut e = entry("test", "1.0", 0, "0.0", "0.0", "0.0", "0.0", "0.0");
        e.protein_g = Grams::from_decimal(Decimal::MAX).unwrap();
        e.servings = Servings::from_u32(2);
        assert!(e.total_protein().is_err());
    }

    #[test]
    fn test_log_entry_append_multiple() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 2).unwrap();

        append_entry(
            dir.path(),
            date,
            &entry("coffee", "2.0", 12, "0.0", "0.0", "0.0", "0.0", "0.0"),
        )?;

        append_entry(
            dir.path(),
            date,
            &entry("oatmeal", "1.0", 418, "22.0", "9.0", "6.0", "60.0", "0.0"),
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day log should exist");
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].title, "coffee");
        assert_eq!(loaded.entries[1].title, "oatmeal");

        Ok(())
    }

    #[test]
    fn test_log_entry_missing_title_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "[[entries]]\nservings = 1.0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());

        Ok(())
    }

    #[test]
    fn test_log_entry_serializes_title_without_slug() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let e = entry(
            "Sour Cream - 60g",
            "1.0",
            60,
            "1.5",
            "0.0",
            "4.0",
            "3.0",
            "0.0",
        );

        append_entry(dir.path(), date, &e)?;
        let content =
            std::fs::read_to_string(dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))))?;
        assert!(content.contains("title = \"Sour Cream - 60g\""));
        assert!(!content.contains("slug"));

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

    #[test]
    fn test_set_exercise_calories_round_trip() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();

        set_exercise_calories(dir.path(), date, Calories::from_str("300.5").unwrap())?;
        let loaded = load_day(dir.path(), date)?.expect("day log should exist");
        assert_eq!(
            loaded.exercise_calories,
            Calories::from_str("300.5").unwrap()
        );
        assert!(loaded.entries.is_empty());

        let content =
            std::fs::read_to_string(dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))))?;
        assert!(content.contains("exercise_calories = 300.5"));

        Ok(())
    }

    #[test]
    fn test_exercise_calories_integer_normalized() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "exercise_calories = 300\n\n[[entries]]\nservings = 1.0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\n",
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day log should exist");
        assert_eq!(loaded.exercise_calories, Calories::from_u32(300));

        Ok(())
    }

    #[test]
    fn test_exercise_calories_negative_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "exercise_calories = -50\n",
        )?;

        assert!(load_day(dir.path(), date).is_err());

        Ok(())
    }
}
