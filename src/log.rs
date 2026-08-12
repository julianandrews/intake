use crate::amount::{Calories, Grams, Macros, Servings};
use crate::config::TimeFormat;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Local, LocalResult, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A full RFC 3339 timestamp in UTC (`2026-08-12T21:45:03Z`), attached to a
/// log entry. Optional per entry: legacy entries and entries written with
/// `write_timestamps` off carry `None`. All construction — from the clock or
/// from explicit `HH:MM` user input — happens in intake, never in the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Timestamp(DateTime<Utc>);

impl Timestamp {
    /// The current instant, captured once per command invocation. Truncated
    /// to whole seconds — stored timestamps are RFC 3339 with seconds
    /// precision, matching explicit `--time` / `retime` stamps.
    pub fn now() -> Timestamp {
        Timestamp(Utc::now().with_nanosecond(0).expect("second truncation"))
    }

    /// Interpret `time` of day on `date` as **local** time and convert to
    /// UTC. An ambiguous or nonexistent local time (DST gap or fall-back) is
    /// an error, never a guessed instant.
    pub fn from_local(date: NaiveDate, time: NaiveTime) -> Result<Timestamp> {
        let naive = date.and_time(time);
        match Local.from_local_datetime(&naive) {
            LocalResult::Single(dt) => Ok(Timestamp(dt.with_timezone(&Utc))),
            LocalResult::Ambiguous(..) => bail!(
                "ambiguous local time {} (DST fall-back) — pick a different time",
                naive
            ),
            LocalResult::None => bail!("nonexistent local time {} (DST gap)", naive),
        }
    }

    /// Format for display in the local timezone, per the configured format.
    pub fn format(&self, format: TimeFormat) -> String {
        let offset = Local.offset_from_utc_datetime(&self.0.naive_utc());
        self.format_at(format, offset)
    }

    /// Format for display at an explicit offset; the display boundary
    /// conversion (local timezone) is applied by the caller.
    fn format_at(&self, format: TimeFormat, offset: chrono::FixedOffset) -> String {
        let local = self.0.with_timezone(&offset);
        match format {
            TimeFormat::H24 => local.format("%H:%M").to_string(),
            TimeFormat::H12 => local.format("%-I:%M %p").to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    pub title: String,
    pub servings: Servings,
    pub calories: Calories,
    pub protein_g: Grams,
    pub fiber_g: Grams,
    pub fat_g: Grams,
    pub carbs_g: Grams,
    pub alcohol_g: Grams,
    pub timestamp: Option<Timestamp>,
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

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DayLog {
    pub entries: Vec<LogEntry>,
    pub exercise_calories: Calories,
}

pub(crate) fn log_path(log_dir: &Path, date: NaiveDate) -> PathBuf {
    log_dir.join(format!("{}.toml", date.format("%Y-%m-%d")))
}

/// Create the log directory and hold the directory lock for the duration of
/// the returned guard, so read-modify-write transactions stay serialized
/// against concurrent writers. The directory inode is stable — never renamed
/// or replaced, unlike the day file itself — so the lock stays correct
/// across the atomic rename below. Released on drop; flock is also released
/// by the kernel if the process dies mid-write, so a crash cannot wedge
/// future writes.
pub(crate) fn lock_log_dir(log_dir: &Path) -> Result<fs::File> {
    fs::create_dir_all(log_dir)
        .with_context(|| format!("failed to create log directory: {}", log_dir.display()))?;
    let dir = fs::File::open(log_dir)
        .with_context(|| format!("failed to open log directory: {}", log_dir.display()))?;
    dir.lock().context("failed to lock log directory")?;
    Ok(dir)
}

pub(crate) fn write_day_locked(log_dir: &Path, date: NaiveDate, day_log: &DayLog) -> Result<()> {
    let path = log_path(log_dir, date);

    let content = toml::to_string(day_log).context("failed to serialize log")?;
    let mut tmp = tempfile::NamedTempFile::new_in(log_dir)
        .with_context(|| format!("failed to create temporary log in: {}", log_dir.display()))?;
    tmp.write_all(content.as_bytes())
        .with_context(|| format!("failed to write temporary log: {}", path.display()))?;
    // Sync the data before the rename so the rename only ever publishes
    // fully-durable content: an OS crash can then never leave a zero-length
    // day log behind (which would parse as an error on every later read).
    tmp.as_file()
        .sync_all()
        .with_context(|| format!("failed to sync temporary log: {}", path.display()))?;
    tmp.persist(&path)
        .with_context(|| format!("failed to write log: {}", path.display()))?;

    Ok(())
}

/// Read-modify-write transaction on the day log for `date`, holding the
/// directory lock across the whole operation and writing atomically.
fn update_day<F>(log_dir: &Path, date: NaiveDate, mutate: F) -> Result<()>
where
    F: FnOnce(&mut DayLog),
{
    let _dir_lock = lock_log_dir(log_dir)?;
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

    write_day_locked(log_dir, date, &day_log)
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

/// Remove the entry at 1-based `index` from the day log for `date`, holding
/// the directory lock across the whole read-modify-write.
///
/// The entry at `index` must equal `expected` exactly: callers read the day
/// log first (e.g. to show a confirmation), so a concurrent modification
/// between that read and this removal is an error instead of silently
/// removing a different entry. Also errors if the day log doesn't exist or
/// the index is out of range. If the removal leaves no entries and no
/// exercise calories, the day file itself is deleted so the day view
/// reports "no entries" instead of an empty table.
pub fn remove_entry(
    log_dir: &Path,
    date: NaiveDate,
    index: usize,
    expected: &LogEntry,
) -> Result<LogEntry> {
    let dir_lock = lock_log_dir(log_dir)?;
    let path = log_path(log_dir, date);

    let day_log: DayLog = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read log: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("failed to parse log: {}", path.display()))?
    } else {
        bail!("no entries for {}", date);
    };

    if index == 0 || index > day_log.entries.len() {
        bail!(
            "entry {} not found — day {} has {}",
            index,
            date,
            entry_count_label(day_log.entries.len())
        );
    }

    let found = &day_log.entries[index - 1];
    if found != expected {
        bail!(
            "entry {} changed since it was listed — day {} was modified concurrently; nothing removed",
            index,
            date
        );
    }

    let mut day_log = day_log;
    let removed = day_log.entries.remove(index - 1);
    if day_log.entries.is_empty() && day_log.exercise_calories == Calories::ZERO {
        fs::remove_file(&path)
            .with_context(|| format!("failed to remove log: {}", path.display()))?;
        // Sync the directory so the unlink is durable, matching the sync
        // before the atomic rename in write_day_locked: a crash must not
        // resurrect an entry the user was told was removed.
        dir_lock
            .sync_all()
            .with_context(|| format!("failed to sync log directory: {}", log_dir.display()))?;
    } else {
        write_day_locked(log_dir, date, &day_log)?;
    }

    Ok(removed)
}

/// Set the timestamp of the entry at 1-based `index` in the day log for
/// `date`, holding the directory lock across the whole read-modify-write.
///
/// The entry at `index` must equal `expected` exactly — callers read the day
/// log first (e.g. to show a confirmation), so a concurrent modification
/// between that read and this write is an error instead of silently stamping
/// a different entry. Also errors if the day log doesn't exist or the index
/// is out of range. Returns the updated entry.
pub fn set_entry_timestamp(
    log_dir: &Path,
    date: NaiveDate,
    index: usize,
    expected: &LogEntry,
    timestamp: Timestamp,
) -> Result<LogEntry> {
    let _dir_lock = lock_log_dir(log_dir)?;
    let path = log_path(log_dir, date);

    let mut day_log: DayLog = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read log: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("failed to parse log: {}", path.display()))?
    } else {
        bail!("no entries for {}", date);
    };

    if index == 0 || index > day_log.entries.len() {
        bail!(
            "entry {} not found — day {} has {}",
            index,
            date,
            entry_count_label(day_log.entries.len())
        );
    }

    let found = &day_log.entries[index - 1];
    if found != expected {
        bail!(
            "entry {} changed since it was listed — day {} was modified concurrently; nothing changed",
            index,
            date
        );
    }

    day_log.entries[index - 1].timestamp = Some(timestamp);
    write_day_locked(log_dir, date, &day_log)?;

    Ok(day_log.entries[index - 1].clone())
}

/// "1 entry" or "N entries", for out-of-range error messages.
pub(crate) fn entry_count_label(count: usize) -> String {
    if count == 1 {
        "1 entry".to_string()
    } else {
        format!("{} entries", count)
    }
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

    /// The six macro values in TOML order: calories, protein, fiber, fat,
    /// carbs, alcohol.
    fn entry(title: &str, servings: &str, macros: [&str; 6]) -> LogEntry {
        LogEntry {
            title: title.to_string(),
            servings: Servings::from_str(servings).unwrap(),
            calories: Calories::from_str(macros[0]).unwrap(),
            protein_g: Grams::from_str(macros[1]).unwrap(),
            fiber_g: Grams::from_str(macros[2]).unwrap(),
            fat_g: Grams::from_str(macros[3]).unwrap(),
            carbs_g: Grams::from_str(macros[4]).unwrap(),
            alcohol_g: Grams::from_str(macros[5]).unwrap(),
            timestamp: None,
        }
    }

    #[test]
    fn test_update_day_waits_for_lock() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();

        let dir_handle = std::fs::File::open(dir.path())?;
        dir_handle.lock()?;

        let (tx, rx) = std::sync::mpsc::channel();
        let dir_path = dir.path().to_path_buf();
        let thread = std::thread::spawn(move || {
            let result = append_entry(
                &dir_path,
                date,
                &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
            );
            tx.send(result).unwrap();
        });

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            rx.try_recv().is_err(),
            "append_entry completed while the lock was held"
        );

        drop(dir_handle);

        assert!(rx.recv().unwrap().is_ok());
        thread.join().unwrap();
        Ok(())
    }

    #[test]
    fn test_log_entry_roundtrip() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let e = entry(
            "oatmeal",
            "1.5",
            ["200", "15.0", "5.0", "2.0", "30.0", "0.0"],
        );

        append_entry(dir.path(), date, &e)?;
        let loaded = load_day(dir.path(), date)?.expect("day log should exist");

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].title, "oatmeal");
        assert_eq!(
            loaded.entries[0].servings,
            Servings::from_str("1.5").unwrap()
        );
        assert_eq!(
            loaded.entries[0].calories,
            Calories::from_str("200").unwrap()
        );
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
        let e = entry("test", "2.0", ["100", "10.0", "5.0", "4.0", "20.0", "3.0"]);
        assert_eq!(
            e.total_calories().unwrap(),
            Calories::from_str("200").unwrap()
        );
        assert_eq!(e.total_protein().unwrap(), Grams::from_str("20.0").unwrap());
        assert_eq!(e.total_fiber().unwrap(), Grams::from_str("10.0").unwrap());
        assert_eq!(e.total_fat().unwrap(), Grams::from_str("8.0").unwrap());
        assert_eq!(e.total_carbs().unwrap(), Grams::from_str("40.0").unwrap());
        assert_eq!(e.total_alcohol().unwrap(), Grams::from_str("6.0").unwrap());
    }

    #[test]
    fn test_totals_fractional_servings() {
        let e = entry("test", "1.5", ["100", "10.0", "0.0", "0.0", "0.0", "0.0"]);
        assert_eq!(
            e.total_calories().unwrap(),
            Calories::from_str("150").unwrap()
        );
        assert_eq!(e.total_protein().unwrap(), Grams::from_str("15.0").unwrap());
    }

    #[test]
    fn test_totals_fractional_calories() {
        let mut e = entry("test", "3.0", ["0", "0.0", "0.0", "0.0", "0.0", "0.0"]);
        e.calories = Calories::from_str("33.333").unwrap();
        assert_eq!(
            e.total_calories().unwrap(),
            Calories::from_str("99.999").unwrap()
        );
    }

    #[test]
    fn test_totals_overflow_errors() {
        let mut e = entry("test", "1.0", ["0", "0.0", "0.0", "0.0", "0.0", "0.0"]);
        e.protein_g = Grams::from_decimal(Decimal::MAX).unwrap();
        e.servings = Servings::from_str("2").unwrap();
        assert!(e.total_protein().is_err());
    }

    #[test]
    fn test_log_entry_append_multiple() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 2).unwrap();

        append_entry(
            dir.path(),
            date,
            &entry("coffee", "2.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;

        append_entry(
            dir.path(),
            date,
            &entry(
                "oatmeal",
                "1.0",
                ["418", "22.0", "9.0", "6.0", "60.0", "0.0"],
            ),
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
    fn test_log_entry_serializes_title_without_food_reference() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let e = entry(
            "Sour Cream - 60g",
            "1.0",
            ["60", "1.5", "0.0", "4.0", "3.0", "0.0"],
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
        assert_eq!(loaded.exercise_calories, Calories::from_str("300").unwrap());

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

    #[test]
    fn test_day_log_unknown_field_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "exercise_calories = 0\nbogus = 1\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());

        Ok(())
    }

    #[test]
    fn test_log_entry_unknown_field_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "exercise_calories = 0\n\n[[entries]]\nservings = 1.0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\nbogus = 1\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());

        Ok(())
    }

    #[test]
    fn test_remove_entry_middle() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;
        append_entry(
            dir.path(),
            date,
            &entry(
                "oatmeal",
                "1.0",
                ["418", "22.0", "9.0", "6.0", "60.0", "0.0"],
            ),
        )?;
        append_entry(
            dir.path(),
            date,
            &entry(
                "chili",
                "2.0",
                ["200", "20.0", "10.0", "8.0", "30.0", "0.0"],
            ),
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        let removed = remove_entry(dir.path(), date, 2, &loaded.entries[1])?;
        assert_eq!(removed.title, "oatmeal");

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].title, "coffee");
        assert_eq!(loaded.entries[1].title, "chili");

        Ok(())
    }

    #[test]
    fn test_remove_entry_last_removes_day_file() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;
        append_entry(
            dir.path(),
            date,
            &entry(
                "chili",
                "1.0",
                ["200", "20.0", "10.0", "8.0", "30.0", "0.0"],
            ),
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert!(remove_entry(dir.path(), date, 1, &loaded.entries[0])?.title == "coffee");
        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries.len(), 1);

        assert!(remove_entry(dir.path(), date, 1, &loaded.entries[0])?.title == "chili");
        assert!(
            load_day(dir.path(), date)?.is_none(),
            "empty day file should be removed"
        );

        Ok(())
    }

    #[test]
    fn test_remove_entry_keeps_day_file_with_exercise() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;
        set_exercise_calories(dir.path(), date, Calories::from_str("300").unwrap())?;

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        remove_entry(dir.path(), date, 1, &loaded.entries[0])?;

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert!(loaded.entries.is_empty());
        assert_eq!(loaded.exercise_calories, Calories::from_str("300").unwrap());

        Ok(())
    }

    #[test]
    fn test_remove_entry_out_of_range_errors() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day should exist");

        let err = remove_entry(dir.path(), date, 2, &loaded.entries[0]).unwrap_err();
        assert!(err.to_string().contains("entry 2 not found"));
        assert!(err.to_string().contains("has 1 entry"));

        let err = remove_entry(dir.path(), date, 0, &loaded.entries[0]).unwrap_err();
        assert!(err.to_string().contains("entry 0 not found"));

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries.len(), 1);

        Ok(())
    }

    #[test]
    fn test_remove_entry_no_day_errors() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        let expected = entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]);
        let err = remove_entry(dir.path(), date, 1, &expected).unwrap_err();
        assert!(err.to_string().contains("no entries"));

        Ok(())
    }

    #[test]
    fn test_remove_entry_rejects_changed_entry() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;
        append_entry(
            dir.path(),
            date,
            &entry(
                "chili",
                "1.0",
                ["200", "20.0", "10.0", "8.0", "30.0", "0.0"],
            ),
        )?;
        append_entry(
            dir.path(),
            date,
            &entry(
                "oatmeal",
                "1.0",
                ["418", "22.0", "9.0", "6.0", "60.0", "0.0"],
            ),
        )?;

        // A concurrent removal of an earlier entry shifts the confirmed
        // index: the entry at index 2 is no longer the chili.
        let expected = load_day(dir.path(), date)?
            .expect("day should exist")
            .entries[1]
            .clone();
        let entries = load_day(dir.path(), date)?.expect("day should exist");
        remove_entry(dir.path(), date, 1, &entries.entries[0])?;

        let err = remove_entry(dir.path(), date, 2, &expected).unwrap_err();
        assert!(err.to_string().contains("changed since it was listed"));
        assert!(err.to_string().contains("nothing removed"));

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].title, "chili");

        Ok(())
    }

    #[test]
    fn test_remove_entry_rejects_modified_entry() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;

        let expected = load_day(dir.path(), date)?
            .expect("day should exist")
            .entries[0]
            .clone();
        let mut changed = expected.clone();
        changed.servings = Servings::from_str("2.0").unwrap();

        let err = remove_entry(dir.path(), date, 1, &changed).unwrap_err();
        assert!(err.to_string().contains("changed since it was listed"));

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(
            loaded.entries[0].servings,
            Servings::from_str("1.0").unwrap()
        );

        Ok(())
    }

    #[test]
    fn test_remove_entry_rejects_timestamp_difference() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;

        // Entry equality includes the timestamp: an expected entry whose
        // only difference is the timestamp must be rejected, matching the
        // design doc's "entry equality now includes the timestamp".
        let expected = load_day(dir.path(), date)?
            .expect("day should exist")
            .entries[0]
            .clone();
        let mut changed = expected.clone();
        changed.timestamp = Some(Timestamp::now());

        let err = remove_entry(dir.path(), date, 1, &changed).unwrap_err();
        assert!(err.to_string().contains("changed since it was listed"));

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].timestamp, None, "file must be untouched");

        Ok(())
    }

    #[test]
    fn test_remove_entry_preserves_remaining_entries() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "2.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        let removed = remove_entry(dir.path(), date, 1, &loaded.entries[0])?;
        assert_eq!(removed.servings, Servings::from_str("2.0").unwrap());

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(
            loaded.entries[0].servings,
            Servings::from_str("1.0").unwrap()
        );

        Ok(())
    }

    fn timestamp(utc: &str) -> Timestamp {
        Timestamp(
            DateTime::parse_from_rfc3339(utc)
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn test_timestamp_serializes_rfc3339_utc() {
        // TOML serializes top-level values only inside tables, so go through
        // a container — the real shape is a `LogEntry` field.
        #[derive(Deserialize, Serialize)]
        struct Holder {
            ts: Option<Timestamp>,
        }
        let ts = timestamp("2026-08-12T21:45:03Z");
        let serialized = toml::to_string(&Holder { ts: Some(ts) }).unwrap();
        assert!(
            serialized.contains("ts = \"2026-08-12T21:45:03Z\""),
            "got: {serialized}"
        );
        let parsed: Holder = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.ts, Some(ts));
    }

    #[test]
    fn test_timestamp_unparseable_rejected() {
        assert!(toml::from_str::<Timestamp>("\"not a timestamp\"").is_err());
        assert!(toml::from_str::<Timestamp>("\"2026-13-40T99:99:99Z\"").is_err());
    }

    #[test]
    fn test_timestamp_format_at_fixed_offsets() {
        let ts = timestamp("2026-08-12T14:05:00Z");
        let utc = chrono::FixedOffset::east_opt(0).unwrap();
        let east2 = chrono::FixedOffset::east_opt(2 * 3600).unwrap();
        let west5 = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(ts.format_at(TimeFormat::H24, utc), "14:05");
        assert_eq!(ts.format_at(TimeFormat::H24, east2), "16:05");
        assert_eq!(ts.format_at(TimeFormat::H24, west5), "09:05");
        assert_eq!(ts.format_at(TimeFormat::H12, east2), "4:05 PM");
        assert_eq!(ts.format_at(TimeFormat::H12, west5), "9:05 AM");
        assert_eq!(
            timestamp("2026-08-12T00:30:00Z").format_at(TimeFormat::H12, east2),
            "2:30 AM"
        );
    }

    #[test]
    fn test_timestamp_from_local_roundtrips_in_local_zone() -> Result<()> {
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let time = NaiveTime::from_hms_opt(14, 30, 0).unwrap();
        let ts = Timestamp::from_local(date, time)?;
        assert_eq!(
            ts.0.with_timezone(&Local).naive_local().date(),
            date,
            "conversion must preserve the local date"
        );
        assert_eq!(ts.0.with_timezone(&Local).naive_local().time(), time);
        // Display roundtrip: formatting at the same instant's local offset
        // must reproduce the input, in any local timezone.
        assert_eq!(ts.format(TimeFormat::H24), "14:30");
        Ok(())
    }

    #[test]
    fn test_log_entry_roundtrip_with_timestamp() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let mut e = entry(
            "oatmeal",
            "1.5",
            ["200", "15.0", "5.0", "2.0", "30.0", "0.0"],
        );
        e.timestamp = Some(timestamp("2026-08-01T14:30:00Z"));
        append_entry(dir.path(), date, &e)?;

        let content =
            std::fs::read_to_string(dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))))?;
        assert!(
            content.contains("timestamp = \"2026-08-01T14:30:00Z\""),
            "got: {content}"
        );

        let loaded = load_day(dir.path(), date)?.expect("day log should exist");
        assert_eq!(loaded.entries[0].timestamp, Some(e.timestamp.unwrap()));
        Ok(())
    }

    #[test]
    fn test_log_entry_malformed_timestamp_rejected() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "exercise_calories = 0\n\n[[entries]]\nservings = 1.0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\ntimestamp = \"yesterday\"\n",
        )?;
        assert!(load_day(dir.path(), date).is_err());
        Ok(())
    }

    #[test]
    fn test_log_entry_absent_timestamp_is_none() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        // A legacy day file written before the timestamp feature: the field
        // is absent entirely, so the entry must load with `None`.
        std::fs::write(
            dir.path().join(format!("{}.toml", date.format("%Y-%m-%d"))),
            "exercise_calories = 0\n\n[[entries]]\nservings = 1.0\ncalories = 12\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\ntitle = \"Coffee\"\n",
        )?;
        let loaded = load_day(dir.path(), date)?.expect("day log should exist");
        assert_eq!(loaded.entries[0].timestamp, None);
        Ok(())
    }

    #[test]
    fn test_set_entry_timestamp_sets_and_persists() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;
        append_entry(
            dir.path(),
            date,
            &entry("chili", "1.0", ["300", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        let ts = Timestamp::from_local(date, NaiveTime::from_hms_opt(19, 45, 0).unwrap())?;
        let updated = set_entry_timestamp(dir.path(), date, 2, &loaded.entries[1], ts)?;
        assert_eq!(updated.timestamp, Some(ts));

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries[0].timestamp, None, "other entries untouched");
        assert_eq!(loaded.entries[1].timestamp, Some(ts));

        // Overwrite an existing timestamp.
        let ts2 = Timestamp::from_local(date, NaiveTime::from_hms_opt(8, 0, 0).unwrap())?;
        set_entry_timestamp(dir.path(), date, 2, &loaded.entries[1], ts2)?;
        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries[1].timestamp, Some(ts2));
        Ok(())
    }

    #[test]
    fn test_set_entry_timestamp_out_of_range_errors() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        let ts = Timestamp::from_local(date, NaiveTime::from_hms_opt(8, 0, 0).unwrap())?;

        let err = set_entry_timestamp(dir.path(), date, 2, &loaded.entries[0], ts).unwrap_err();
        assert!(err.to_string().contains("entry 2 not found"));
        let err = set_entry_timestamp(dir.path(), date, 0, &loaded.entries[0], ts).unwrap_err();
        assert!(err.to_string().contains("entry 0 not found"));
        Ok(())
    }

    #[test]
    fn test_set_entry_timestamp_no_day_errors() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        let expected = entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]);
        let ts = Timestamp::from_local(date, NaiveTime::from_hms_opt(8, 0, 0).unwrap())?;
        let err = set_entry_timestamp(dir.path(), date, 1, &expected, ts).unwrap_err();
        assert!(err.to_string().contains("no entries"));
        Ok(())
    }

    #[test]
    fn test_set_entry_timestamp_rejects_changed_entry() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;
        append_entry(
            dir.path(),
            date,
            &entry("chili", "1.0", ["300", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;
        append_entry(
            dir.path(),
            date,
            &entry("oatmeal", "1.0", ["200", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;

        // A concurrent removal shifts the confirmed index: the entry at
        // index 2 is no longer the chili.
        let expected = load_day(dir.path(), date)?
            .expect("day should exist")
            .entries[1]
            .clone();
        let entries = load_day(dir.path(), date)?.expect("day should exist");
        remove_entry(dir.path(), date, 1, &entries.entries[0])?;

        let ts = Timestamp::from_local(date, NaiveTime::from_hms_opt(8, 0, 0).unwrap())?;
        let err = set_entry_timestamp(dir.path(), date, 2, &expected, ts).unwrap_err();
        assert!(err.to_string().contains("changed since it was listed"));
        assert!(err.to_string().contains("nothing changed"));

        let loaded = load_day(dir.path(), date)?.expect("day should exist");
        assert_eq!(loaded.entries[0].title, "chili");
        assert_eq!(loaded.entries[0].timestamp, None);
        Ok(())
    }

    #[test]
    fn test_set_entry_timestamp_waits_for_lock() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
        append_entry(
            dir.path(),
            date,
            &entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        )?;

        let dir_handle = std::fs::File::open(dir.path())?;
        dir_handle.lock()?;

        let (tx, rx) = std::sync::mpsc::channel();
        let dir_path = dir.path().to_path_buf();
        let ts = Timestamp::from_local(date, NaiveTime::from_hms_opt(8, 0, 0).unwrap())?;
        let entry = load_day(dir.path(), date)?
            .expect("day should exist")
            .entries[0]
            .clone();
        let thread = std::thread::spawn(move || {
            let result = set_entry_timestamp(&dir_path, date, 1, &entry, ts);
            tx.send(result).unwrap();
        });

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            rx.try_recv().is_err(),
            "set_entry_timestamp completed while the lock was held"
        );

        drop(dir_handle);

        assert!(rx.recv().unwrap().is_ok());
        thread.join().unwrap();
        Ok(())
    }
}
