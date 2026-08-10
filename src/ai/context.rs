use crate::amount::{Calories, Macros};
use crate::config::{Column, Config};
use crate::{food, log};
use anyhow::{Context as AnyhowContext, Result};
use chrono::{Days, NaiveDate};
use std::collections::HashMap;
use std::path::Path;

use super::catalog;

pub const DEFAULT_HISTORY_DAYS: u32 = 14;
const HISTORY_MAX_LINES: usize = 200;

pub fn entry_line(entry: &log::LogEntry) -> String {
    format!(
        "{} | {} | {}, {}, {}, {}, {}, {}",
        entry.title,
        entry.servings,
        entry.calories,
        entry.protein_g,
        entry.fiber_g,
        entry.fat_g,
        entry.carbs_g,
        entry.alcohol_g
    )
}

pub fn totals_line(day: &log::DayLog, config: &Config) -> Result<String> {
    let mut totals = Macros::ZERO;
    for entry in &day.entries {
        totals = totals
            .checked_add(&entry.totals()?)
            .context("day macro total overflow")?;
    }
    let mut line = format!(
        "totals: {} | {} | {} | {} | {} | {}",
        totals.calories,
        totals.protein_g,
        totals.fiber_g,
        totals.fat_g,
        totals.carbs_g,
        totals.alcohol_g
    );
    if day.exercise_calories > Calories::ZERO {
        line.push_str(&format!(" | exercise: {}", day.exercise_calories));
    }
    let targets = config.targets()?;
    let mut parts: Vec<String> = Vec::new();
    for (name, column) in [
        ("calories", Column::Calories),
        ("protein", Column::Protein),
        ("fiber", Column::Fiber),
        ("fat", Column::Fat),
        ("carbs", Column::Carbs),
        ("alcohol", Column::Alcohol),
    ] {
        let t = targets.for_column(column);
        match (t.min, t.max) {
            (Some(min), Some(max)) => parts.push(format!("{name} {min}-{max}")),
            (Some(min), None) => parts.push(format!("{name} min {min}")),
            (None, Some(max)) => parts.push(format!("{name} max {max}")),
            (None, None) => {}
        }
    }
    if !parts.is_empty() {
        line.push_str(&format!(" | targets: {}", parts.join(", ")));
    }
    Ok(line)
}

pub fn history_digest(log_dir: &Path, end_date: NaiveDate, history_days: u32) -> Result<String> {
    let mut lines: Vec<(usize, String)> = Vec::new();
    let start = end_date
        .checked_sub_days(Days::new(history_days as u64))
        .unwrap_or(end_date);
    let mut d = start;
    while d < end_date {
        if let Some(day) = log::load_day(log_dir, d)? {
            for entry in &day.entries {
                lines.push((lines.len(), entry_line(entry)));
            }
        }
        d = d
            .checked_add_days(Days::new(1))
            .context("history window date overflow")?;
    }
    if lines.len() > HISTORY_MAX_LINES {
        lines = lines.split_off(lines.len() - HISTORY_MAX_LINES);
    }

    let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
    for (seq, line) in lines {
        let count = counts.entry(line).or_insert((0, 0));
        count.0 += 1;
        count.1 = count.1.max(seq);
    }
    let mut entries: Vec<(&String, usize, usize)> = counts
        .iter()
        .map(|(line, (count, seq))| (line, *count, *seq))
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));

    if entries.is_empty() {
        return Ok(String::new());
    }
    let mut out = format!("History ({history_days} days before {end_date}):\n");
    for (line, count, _) in entries {
        out.push_str(&format!("  {line} ×{count}\n"));
    }
    Ok(out)
}

pub fn day_context(
    date: NaiveDate,
    day: Option<&log::DayLog>,
    log_dir: &Path,
    config: &Config,
) -> Result<String> {
    let empty = log::DayLog {
        entries: Vec::new(),
        exercise_calories: Calories::ZERO,
    };
    let day = day.unwrap_or(&empty);

    let mut out = format!("Day {date}:\n");
    if day.entries.is_empty() {
        out.push_str("  (no entries)\n");
    } else {
        for (i, entry) in day.entries.iter().enumerate() {
            out.push_str(&format!("  {}. {}\n", i + 1, entry_line(entry)));
        }
    }
    out.push_str(&format!("  {}\n", totals_line(day, config)?));

    let digest = history_digest(log_dir, date, history_days(config))?;
    if !digest.is_empty() {
        out.push('\n');
        out.push_str(&digest);
    }
    Ok(out)
}

pub fn history_days(config: &Config) -> u32 {
    config
        .ai
        .as_ref()
        .and_then(|ai| ai.history_days)
        .unwrap_or(DEFAULT_HISTORY_DAYS)
}

fn is_complex(p: &(String, food::Food)) -> bool {
    p.1.ingredients.len() >= 3
}

fn has_notes(p: &(String, food::Food)) -> bool {
    !p.1.notes.trim().is_empty()
}

fn is_simple(p: &(String, food::Food)) -> bool {
    p.1.ingredients.len() <= 2
}

type SlotPred = fn(&(String, food::Food)) -> bool;

pub fn sample_foods(foods_dir: &Path) -> Result<Vec<food::Food>> {
    let pairs = catalog::find_all_foods_with_names(foods_dir)?;
    let slots: [SlotPred; 3] = [is_complex, has_notes, is_simple];
    let mut picked: Vec<String> = Vec::new();
    let mut out: Vec<food::Food> = Vec::new();
    for slot in slots {
        if let Some(p) = pairs.iter().find(|p| slot(p) && !picked.contains(&p.0)) {
            picked.push(p.0.clone());
            out.push(p.1.clone());
        } else if let Some(p) = pairs.iter().find(|p| !picked.contains(&p.0)) {
            picked.push(p.0.clone());
            out.push(p.1.clone());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn grams(value: &str) -> crate::amount::Grams {
        crate::amount::Grams::from_str(value).unwrap()
    }

    fn entry(title: &str, servings: &str, calories: &str) -> log::LogEntry {
        log::LogEntry {
            title: title.to_string(),
            servings: crate::amount::Servings::from_str(servings).unwrap(),
            calories: Calories::from_str(calories).unwrap(),
            protein_g: grams("0"),
            fiber_g: grams("0"),
            fat_g: grams("0"),
            carbs_g: grams("0"),
            alcohol_g: grams("0"),
        }
    }

    fn day(entries: Vec<log::LogEntry>) -> log::DayLog {
        log::DayLog {
            entries,
            exercise_calories: Calories::from_str("0").unwrap(),
        }
    }

    fn write_day(dir: &Path, date: &str, entries: Vec<log::LogEntry>) {
        let day = day(entries);
        let content = toml::to_string(&day).unwrap();
        std::fs::write(dir.join(format!("{date}.toml")), content).unwrap();
    }

    #[test]
    fn test_entry_line_format() {
        let e = log::LogEntry {
            title: "Cherries - 155g".to_string(),
            servings: crate::amount::Servings::from_str("1.5").unwrap(),
            calories: Calories::from_str("100").unwrap(),
            protein_g: grams("1.5"),
            fiber_g: grams("3.0"),
            fat_g: grams("0.5"),
            carbs_g: grams("24.0"),
            alcohol_g: grams("0.0"),
        };
        assert_eq!(
            entry_line(&e),
            "Cherries - 155g | 1.5 | 100, 1.5, 3, 0.5, 24, 0"
        );
    }

    #[test]
    fn test_totals_line_sums_and_exercise() {
        let d = log::DayLog {
            entries: vec![entry("coffee", "2", "100"), entry("chili", "1", "300")],
            exercise_calories: Calories::from_str("200").unwrap(),
        };
        let config = Config::default();
        let line = totals_line(&d, &config).unwrap();
        assert!(line.starts_with("totals: 500 | 0 | 0 | 0 | 0 | 0 | exercise: 200"));
    }

    #[test]
    fn test_totals_line_with_targets() {
        let config: Config = toml::from_str(
            "max_calories = 2000\nmin_protein = 100\nmin_fiber = 30\nmaintenance_calories = 2400\n",
        )
        .unwrap();
        let d = day(vec![]);
        let line = totals_line(&d, &config).unwrap();
        assert!(line.contains("calories max 2000"));
        assert!(line.contains("protein min 100"));
        assert!(line.contains("fiber min 30"));
        assert!(!line.contains("exercise"));
        assert!(!line.contains("fat min"));
    }

    #[test]
    fn test_totals_line_band_targets() {
        let config: Config = toml::from_str("min_fat = 50\nmax_fat = 90\n").unwrap();
        let d = day(vec![]);
        let line = totals_line(&d, &config).unwrap();
        assert!(line.contains("fat 50-90"));
    }

    #[test]
    fn test_history_digest_anchors_before_end_date() {
        let dir = tempfile::TempDir::new().unwrap();
        write_day(dir.path(), "2026-08-08", vec![entry("coffee", "1", "12")]);
        write_day(
            dir.path(),
            "2026-08-09",
            vec![entry("coffee", "1", "12"), entry("chili", "1", "300")],
        );
        let end = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        let digest = history_digest(dir.path(), end, 14).unwrap();
        assert!(digest.contains("History (14 days before 2026-08-10):"));
        assert!(digest.contains("coffee | 1 | 12, 0, 0, 0, 0, 0 ×2"));
        assert!(digest.contains("chili | 1 | 300, 0, 0, 0, 0, 0 ×1"));
    }

    #[test]
    fn test_history_digest_excludes_end_date() {
        let dir = tempfile::TempDir::new().unwrap();
        write_day(dir.path(), "2026-08-10", vec![entry("coffee", "1", "12")]);
        let end = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        let digest = history_digest(dir.path(), end, 14).unwrap();
        assert!(digest.is_empty());
    }

    #[test]
    fn test_history_digest_empty_window() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        assert!(history_digest(dir.path(), end, 0).unwrap().is_empty());
    }

    #[test]
    fn test_history_digest_count_sort_ties_most_recent_first() {
        let dir = tempfile::TempDir::new().unwrap();
        write_day(dir.path(), "2026-08-08", vec![entry("coffee", "1", "12")]);
        write_day(
            dir.path(),
            "2026-08-09",
            vec![entry("coffee", "1", "12"), entry("chili", "1", "300")],
        );
        let end = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        let digest = history_digest(dir.path(), end, 14).unwrap();
        let lines: Vec<&str> = digest.lines().skip(1).collect();
        assert!(lines[0].contains("coffee"));
        assert!(lines[1].contains("chili"));
    }

    #[test]
    fn test_history_digest_caps_raw_window_before_dedup() {
        let dir = tempfile::TempDir::new().unwrap();
        write_day(
            dir.path(),
            "2026-08-08",
            vec![entry("oldest", "1", "1"), entry("middle", "1", "2")],
        );
        write_day(dir.path(), "2026-08-09", vec![entry("newest", "1", "3")]);
        let end = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        let digest = history_digest(dir.path(), end, 14).unwrap();
        assert!(digest.contains("newest"));
        assert!(digest.contains("middle"));
        assert!(digest.contains("oldest"));
    }

    #[test]
    fn test_history_digest_drops_oldest_beyond_cap() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut entries = Vec::new();
        for i in 0..250 {
            entries.push(entry(&format!("item-{i}"), "1", "1"));
        }
        write_day(dir.path(), "2026-08-09", entries);
        let end = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        let digest = history_digest(dir.path(), end, 14).unwrap();
        assert!(!digest.contains("item-0"));
        assert!(digest.contains("item-249"));
        assert!(digest.contains("×250") || digest.contains(" ×"));
        let count_lines = digest.lines().count();
        assert!(count_lines <= 202);
    }

    #[test]
    fn test_day_context_numbered_rows_and_totals() {
        let dir = tempfile::TempDir::new().unwrap();
        write_day(
            dir.path(),
            "2026-08-09",
            vec![entry("coffee", "1", "12"), entry("chili", "1", "300")],
        );
        let d = log::load_day(dir.path(), NaiveDate::from_ymd_opt(2026, 8, 9).unwrap())
            .unwrap()
            .unwrap();
        let config = Config::default();
        let ctx = day_context(
            NaiveDate::from_ymd_opt(2026, 8, 10).unwrap(),
            Some(&d),
            dir.path(),
            &config,
        )
        .unwrap();
        assert!(ctx.contains("Day 2026-08-10:"));
        assert!(ctx.contains("  1. coffee | 1 | 12, 0, 0, 0, 0, 0"));
        assert!(ctx.contains("  2. chili | 1 | 300, 0, 0, 0, 0, 0"));
        assert!(ctx.contains("totals: 312"));
    }

    #[test]
    fn test_day_context_empty_day() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config::default();
        let ctx = day_context(
            NaiveDate::from_ymd_opt(2026, 8, 10).unwrap(),
            None,
            dir.path(),
            &config,
        )
        .unwrap();
        assert!(ctx.contains("(no entries)"));
        assert!(ctx.contains("totals: 0"));
        assert!(!ctx.contains("History"));
    }

    #[test]
    fn test_day_context_includes_history_digest() {
        let dir = tempfile::TempDir::new().unwrap();
        write_day(dir.path(), "2026-08-09", vec![entry("coffee", "1", "12")]);
        let config = Config::default();
        let ctx = day_context(
            NaiveDate::from_ymd_opt(2026, 8, 10).unwrap(),
            None,
            dir.path(),
            &config,
        )
        .unwrap();
        assert!(ctx.contains("History (14 days before 2026-08-10):"));
        assert!(ctx.contains("coffee | 1 | 12, 0, 0, 0, 0, 0 ×1"));
    }

    #[test]
    fn test_history_days_default() {
        assert_eq!(history_days(&Config::default()), 14);
    }

    #[test]
    fn test_history_days_from_config() {
        let config: Config = toml::from_str("[ai]\nhistory_days = 7\n").unwrap();
        assert_eq!(history_days(&config), 7);
    }

    fn foods_dir() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let foods = [
            (
                "complex.toml",
                "title = \"Complex\"\nservings = 2\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n\n[[ingredients]]\nname = \"B\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n\n[[ingredients]]\nname = \"C\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
            ),
            (
                "noted.toml",
                "title = \"Noted\"\nservings = 1\nnotes = \"Best warm\"\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
            ),
            (
                "simple.toml",
                "title = \"Simple\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
            ),
            (
                "simple2.toml",
                "title = \"Simple 2\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
            ),
        ];
        for (name, toml) in foods {
            std::fs::write(dir.path().join(name), toml).unwrap();
        }
        dir
    }

    #[test]
    fn test_sample_foods_diversity_slots() {
        let dir = foods_dir();
        let samples = sample_foods(dir.path()).unwrap();
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].title, "Complex");
        assert_eq!(samples[1].title, "Noted");
        assert_eq!(samples[2].title, "Simple");
    }

    #[test]
    fn test_sample_foods_no_duplicates_when_slots_overlap() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("big-note.toml"),
            "title = \"Big Note\"\nservings = 1\nnotes = \"Warm\"\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n\n[[ingredients]]\nname = \"B\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n\n[[ingredients]]\nname = \"C\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("other.toml"),
            "title = \"Other\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        )
        .unwrap();
        let samples = sample_foods(dir.path()).unwrap();
        let titles: Vec<&str> = samples.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles.len(), 2);
        assert!(titles.contains(&"Big Note"));
        assert!(titles.contains(&"Other"));
    }

    #[test]
    fn test_sample_foods_small_catalog() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("one.toml"),
            "title = \"One\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        )
        .unwrap();
        let samples = sample_foods(dir.path()).unwrap();
        assert_eq!(samples.len(), 1);
    }

    #[test]
    fn test_sample_foods_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let samples = sample_foods(dir.path()).unwrap();
        assert!(samples.is_empty());
    }
}
