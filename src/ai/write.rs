use crate::amount::Calories;
use crate::log::{lock_log_dir, log_path, write_day_locked, DayLog};
use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use std::fs;
use std::path::Path;

/// A day log produced by `apply_ops` plus the count of add ops that were
/// applied — the additions the day carries. The count lets the write path
/// stamp exactly the added entries with timestamps; `apply_ops` itself stays
/// pure.
pub(crate) struct AppliedDay {
    pub day: DayLog,
    pub add_ops: usize,
}

/// Stamp the entries added by add ops with `now` (UTC). `apply_ops` appends
/// all additions at the end, so they are exactly the trailing `add_ops`
/// entries of the applied day. Existing entries — including rows rewritten
/// by replace ops, which edit content rather than re-logging — keep their
/// timestamps. Stamping happens at write time, after confirmation, so the
/// stamp records when the write landed.
pub(crate) fn stamp_added_entries(applied: AppliedDay, now: crate::log::Timestamp) -> AppliedDay {
    let mut day = applied.day;
    let k = applied.add_ops;
    if k > 0 {
        // `apply_ops` appends exactly one entry per add op, so the additions
        // are the trailing `k` entries; assert the invariant rather than
        // underflowing the slice start if it ever breaks.
        debug_assert!(k <= day.entries.len());
        let start = day.entries.len() - k;
        for entry in &mut day.entries[start..] {
            entry.timestamp = Some(now);
        }
    }
    AppliedDay {
        day,
        add_ops: applied.add_ops,
    }
}

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
            timestamp: None,
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

    fn now() -> crate::log::Timestamp {
        crate::log::Timestamp::now()
    }

    fn applied(entries: Vec<crate::log::LogEntry>, add_ops: usize) -> AppliedDay {
        AppliedDay {
            day: DayLog {
                entries,
                exercise_calories: Calories::ZERO,
            },
            add_ops,
        }
    }

    #[test]
    fn test_stamp_added_entries_stamps_exactly_trailing_additions() {
        let t0 = now();
        let mut entries = vec![
            entry("coffee", "1.0", ["12", "0.0", "0.0", "0.0", "0.0", "0.0"]),
            entry("chili", "1.0", ["300", "0.0", "0.0", "0.0", "0.0", "0.0"]),
            entry("oatmeal", "1.0", ["200", "0.0", "0.0", "0.0", "0.0", "0.0"]),
            entry("apple", "1.0", ["52", "0.0", "0.0", "0.0", "0.0", "0.0"]),
        ];
        // A pre-existing timed row must keep its stamp untouched.
        entries[0].timestamp = Some(t0);
        let day = applied(entries, 2);
        let t = now();
        let stamped = stamp_added_entries(day, t);
        assert_eq!(stamped.add_ops, 2);
        assert_eq!(stamped.day.entries[0].timestamp, Some(t0));
        assert_eq!(stamped.day.entries[1].timestamp, None);
        assert_eq!(stamped.day.entries[2].timestamp, Some(t));
        assert_eq!(stamped.day.entries[3].timestamp, Some(t));
    }

    #[test]
    fn test_stamp_added_entries_no_ops_is_noop() {
        let day = applied(
            vec![entry(
                "coffee",
                "1.0",
                ["12", "0.0", "0.0", "0.0", "0.0", "0.0"],
            )],
            0,
        );
        let t = now();
        let stamped = stamp_added_entries(day, t);
        assert_eq!(stamped.day.entries[0].timestamp, None);
        assert_eq!(stamped.day.entries.len(), 1);
    }

    #[test]
    fn test_stamp_added_entries_empty_day() {
        let day = applied(vec![], 0);
        let stamped = stamp_added_entries(day, now());
        assert!(stamped.day.entries.is_empty());
    }
}
