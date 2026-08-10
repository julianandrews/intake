use crate::amount::Calories;
use crate::log::{lock_log_dir, log_path, write_day_locked, DayLog};
use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use std::fs;
use std::path::Path;

/// Write `new` as the day log for `date`, but only if the current file still
/// matches `expected` exactly (or both are absent). Runs inside the log
/// directory lock, so the check and the write are atomic against concurrent
/// writers — a day changed since the caller's context was built aborts
/// instead of being overwritten. An applied day with no entries and no
/// exercise calories deletes the day file instead of writing one, matching
/// [`crate::log::remove_entry`].
pub(crate) fn write_day_checked(
    log_dir: &Path,
    date: NaiveDate,
    expected: Option<&DayLog>,
    new: DayLog,
) -> Result<()> {
    let dir_lock = lock_log_dir(log_dir)?;
    let path = log_path(log_dir, date);

    let current: Option<DayLog> = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read log: {}", path.display()))?;
        Some(
            toml::from_str(&content)
                .with_context(|| format!("failed to parse log: {}", path.display()))?,
        )
    } else {
        None
    };

    if current.as_ref() != expected {
        bail!(
            "day {} changed since this proposal was generated — re-run",
            date
        );
    }

    if new.entries.is_empty() && new.exercise_calories == Calories::ZERO {
        if current.is_some() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove log: {}", path.display()))?;
            // Sync the directory so the unlink is durable, matching the sync
            // before the atomic rename in write_day_locked.
            dir_lock
                .sync_all()
                .with_context(|| format!("failed to sync log directory: {}", log_dir.display()))?;
        }
        return Ok(());
    }

    write_day_locked(log_dir, date, &new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::{Calories, Grams, Servings};
    use crate::log::load_day;
    use std::str::FromStr;

    fn entry(title: &str, servings: &str, macros: [&str; 6]) -> crate::log::LogEntry {
        crate::log::LogEntry {
            title: title.to_string(),
            servings: Servings::from_str(servings).unwrap(),
            calories: Calories::from_str(macros[0]).unwrap(),
            protein_g: Grams::from_str(macros[1]).unwrap(),
            fiber_g: Grams::from_str(macros[2]).unwrap(),
            fat_g: Grams::from_str(macros[3]).unwrap(),
            carbs_g: Grams::from_str(macros[4]).unwrap(),
            alcohol_g: Grams::from_str(macros[5]).unwrap(),
        }
    }

    #[test]
    fn test_write_day_checked_creates_and_updates() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();

        let original = DayLog {
            entries: vec![entry(
                "coffee",
                "1.0",
                ["12", "0.0", "0.0", "0.0", "0.0", "0.0"],
            )],
            exercise_calories: Calories::ZERO,
        };
        let updated = DayLog {
            entries: vec![entry(
                "chili",
                "1.0",
                ["300", "0.0", "0.0", "0.0", "0.0", "0.0"],
            )],
            exercise_calories: Calories::from_str("200").unwrap(),
        };

        write_day_checked(dir.path(), date, None, original.clone())?;
        assert_eq!(load_day(dir.path(), date)?.as_ref(), Some(&original));

        write_day_checked(dir.path(), date, Some(&original), updated.clone())?;
        assert_eq!(load_day(dir.path(), date)?.as_ref(), Some(&updated));

        Ok(())
    }

    #[test]
    fn test_write_day_checked_stale_expected_aborts() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();

        let original = DayLog {
            entries: vec![entry(
                "coffee",
                "1.0",
                ["12", "0.0", "0.0", "0.0", "0.0", "0.0"],
            )],
            exercise_calories: Calories::ZERO,
        };
        write_day_checked(dir.path(), date, None, original.clone())?;

        let changed = DayLog {
            entries: vec![entry(
                "chili",
                "1.0",
                ["300", "0.0", "0.0", "0.0", "0.0", "0.0"],
            )],
            exercise_calories: Calories::ZERO,
        };
        write_day_checked(dir.path(), date, Some(&original), changed.clone())?;

        let err =
            write_day_checked(dir.path(), date, Some(&original), original.clone()).unwrap_err();
        assert!(err.to_string().contains("changed since this proposal"));

        assert_eq!(load_day(dir.path(), date)?.as_ref(), Some(&changed));

        Ok(())
    }

    #[test]
    fn test_write_day_checked_file_appeared_aborts() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();

        let day = DayLog {
            entries: vec![entry(
                "coffee",
                "1.0",
                ["12", "0.0", "0.0", "0.0", "0.0", "0.0"],
            )],
            exercise_calories: Calories::ZERO,
        };
        write_day_checked(dir.path(), date, None, day.clone())?;

        let err = write_day_checked(dir.path(), date, None, day.clone()).unwrap_err();
        assert!(err.to_string().contains("changed since this proposal"));

        Ok(())
    }

    #[test]
    fn test_write_day_checked_empty_day_deletes_file() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();

        let original = DayLog {
            entries: vec![entry(
                "coffee",
                "1.0",
                ["12", "0.0", "0.0", "0.0", "0.0", "0.0"],
            )],
            exercise_calories: Calories::ZERO,
        };
        write_day_checked(dir.path(), date, None, original.clone())?;

        let empty = DayLog {
            entries: Vec::new(),
            exercise_calories: Calories::ZERO,
        };
        write_day_checked(dir.path(), date, Some(&original), empty.clone())?;
        assert_eq!(load_day(dir.path(), date)?, None);

        Ok(())
    }

    #[test]
    fn test_write_day_checked_empty_new_with_no_file_is_noop() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();

        let empty = DayLog {
            entries: Vec::new(),
            exercise_calories: Calories::ZERO,
        };
        write_day_checked(dir.path(), date, None, empty)?;
        assert_eq!(load_day(dir.path(), date)?, None);

        Ok(())
    }

    #[test]
    fn test_write_day_checked_empty_day_keeps_file_with_exercise() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();

        let original = DayLog {
            entries: vec![entry(
                "coffee",
                "1.0",
                ["12", "0.0", "0.0", "0.0", "0.0", "0.0"],
            )],
            exercise_calories: Calories::from_str("300").unwrap(),
        };
        write_day_checked(dir.path(), date, None, original.clone())?;

        let empty_with_exercise = DayLog {
            entries: Vec::new(),
            exercise_calories: Calories::from_str("300").unwrap(),
        };
        write_day_checked(
            dir.path(),
            date,
            Some(&original),
            empty_with_exercise.clone(),
        )?;
        assert_eq!(
            load_day(dir.path(), date)?.as_ref(),
            Some(&empty_with_exercise)
        );

        Ok(())
    }

    #[test]
    fn test_write_day_checked_waits_for_lock() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();

        let dir_handle = std::fs::File::open(dir.path())?;
        dir_handle.lock()?;

        let day = DayLog {
            entries: vec![entry(
                "coffee",
                "1.0",
                ["12", "0.0", "0.0", "0.0", "0.0", "0.0"],
            )],
            exercise_calories: Calories::ZERO,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let dir_path = dir.path().to_path_buf();
        let thread = std::thread::spawn(move || {
            let result = write_day_checked(&dir_path, date, None, day);
            tx.send(result).unwrap();
        });

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            rx.try_recv().is_err(),
            "write_day_checked completed while the lock was held"
        );

        drop(dir_handle);

        assert!(rx.recv().unwrap().is_ok());
        thread.join().unwrap();
        Ok(())
    }
}
