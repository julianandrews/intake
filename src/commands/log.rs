use crate::amount::{Calories, Macros, Servings};
use crate::config::{Column, Config, TimeFormat};
use crate::confirm;
use crate::display;
use crate::display::{Align, ColumnValue, Table};
use crate::log::Timestamp;
use crate::{food, log};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{Local, NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;

pub(crate) fn cmd_exercise(
    writer: &mut impl Write,
    log_dir: &Path,
    date: chrono::NaiveDate,
    calories: Calories,
) -> Result<()> {
    log::set_exercise_calories(log_dir, date, calories)?;
    writeln!(
        writer,
        "Recorded {} exercise calories for {}",
        calories, date
    )?;
    Ok(())
}

/// Remove entry `index` (1-based, as shown in the `#` column of the day
/// view, `intake`) from the day log for `date`, then show the updated day.
pub(crate) fn cmd_rm(
    writer: &mut impl Write,
    log_dir: &Path,
    date: chrono::NaiveDate,
    index: u32,
    yes: bool,
    config: &Config,
) -> Result<()> {
    let day_log =
        log::load_day(log_dir, date)?.with_context(|| format!("no entries for {}", date))?;
    if index as usize > day_log.entries.len() {
        bail!(
            "entry {} not found — day {} has {}",
            index,
            date,
            log::entry_count_label(day_log.entries.len())
        );
    }

    let entry = &day_log.entries[index as usize - 1];
    let label = entry_label(index, entry, date)?;

    if !yes {
        match confirm::confirm_yes_no(&format!("Remove {}?", label))? {
            Some(true) => {}
            Some(false) => {
                writeln!(writer, "Nothing removed")?;
                return Ok(());
            }
            None => return confirm::nothing_confirmed(writer, "removed"),
        }
    }

    // `remove_entry` revalidates `entry` against the day file under the lock:
    // if the day changed since the confirmation read, the removal aborts
    // instead of silently removing a different entry.
    log::remove_entry(log_dir, date, index as usize, entry)?;
    writeln!(writer, "Removed {}", label)?;
    writeln!(writer)?;
    cmd_day(writer, log_dir, date, config)?;

    Ok(())
}

/// "entry N (Title, X serving(s), Y kcal) from D": Y is the row's total
/// calories (per-serving scaled by servings), matching the day table's
/// calories column.
fn entry_label(index: u32, entry: &log::LogEntry, date: chrono::NaiveDate) -> Result<String> {
    Ok(format!(
        "entry {} ({}, {} {}, {} kcal) from {}",
        index,
        entry.title,
        entry.servings,
        servings_word(entry.servings),
        entry.total_calories()?,
        date
    ))
}

/// Set the timestamp of entry `index` (1-based, as shown in the `#` column
/// of the day view, `intake`) to `time` of day on `date` (local), then show
/// the updated day. Mirrors `cmd_rm`'s confirm flow; the write revalidates
/// the entry against the day file under the lock, so a concurrent change
/// aborts instead of stamping a different entry.
pub(crate) fn cmd_retime(
    writer: &mut impl Write,
    log_dir: &Path,
    date: NaiveDate,
    index: u32,
    time: NaiveTime,
    yes: bool,
    config: &Config,
) -> Result<()> {
    let day_log =
        log::load_day(log_dir, date)?.with_context(|| format!("no entries for {}", date))?;
    if index as usize > day_log.entries.len() {
        bail!(
            "entry {} not found — day {} has {}",
            index,
            date,
            log::entry_count_label(day_log.entries.len())
        );
    }

    let entry = &day_log.entries[index as usize - 1];
    let timestamp = Timestamp::from_local(date, time)?;
    let format = config.time_format();
    let label = entry_label(index, entry, date)?;
    let prompt = format!("Set {} to {}?", label, timestamp.format(format));

    if !yes {
        match confirm::confirm_yes_no(&prompt)? {
            Some(true) => {}
            Some(false) => {
                writeln!(writer, "Nothing changed")?;
                return Ok(());
            }
            None => return confirm::nothing_confirmed(writer, "changed"),
        }
    }

    // `set_entry_timestamp` revalidates `entry` against the day file under
    // the lock: if the day changed since the confirmation read, the write
    // aborts instead of stamping a different entry.
    log::set_entry_timestamp(log_dir, date, index as usize, entry, timestamp)?;
    writeln!(writer, "Set {} to {}", label, timestamp.format(format))?;
    writeln!(writer)?;
    cmd_day(writer, log_dir, date, config)?;

    Ok(())
}

/// "serving" for exactly one, "servings" otherwise.
fn servings_word(servings: Servings) -> &'static str {
    if servings == Servings::ONE {
        "serving"
    } else {
        "servings"
    }
}

/// The user's `log` request: the food-or-adhoc decision is made before the
/// command runs (macro-flag presence wins), so `adhoc` selects the path and
/// `macros` supplies the ad-hoc values. `time` is an explicit time of day to
/// stamp the entry with, instead of now.
pub(crate) struct LogRequest<'a> {
    pub name: &'a str,
    pub servings: Servings,
    pub macros: &'a Macros,
    pub adhoc: bool,
    pub time: Option<NaiveTime>,
}

/// The timestamp for a new entry: an explicit `--time` (local time on the
/// target date) always wins over the `write_timestamps` toggle; otherwise
/// `now` when the toggle is on, `None` when it is off.
fn entry_timestamp(
    date: NaiveDate,
    time: Option<NaiveTime>,
    config: &Config,
) -> Result<Option<Timestamp>> {
    if let Some(time) = time {
        return Ok(Some(Timestamp::from_local(date, time)?));
    }
    Ok(config.write_timestamps().then(Timestamp::now))
}

/// Log an entry for `date`: the food path when no macro flag was given (the
/// name must resolve to a food name, macros computed from its file), the
/// ad-hoc path when any macro flag was given (name is the title, macros as
/// given, zeros for the rest).
pub(crate) fn cmd_log(
    writer: &mut impl Write,
    foods_dir: &Path,
    log_dir: &Path,
    request: LogRequest<'_>,
    date: chrono::NaiveDate,
    config: &Config,
) -> Result<()> {
    let LogRequest {
        name,
        servings,
        macros,
        adhoc,
        time,
    } = request;
    let timestamp = entry_timestamp(date, time, config)?;
    if adhoc {
        let entry = log::LogEntry {
            title: name.to_string(),
            servings,
            calories: macros.calories,
            protein_g: macros.protein_g,
            fiber_g: macros.fiber_g,
            fat_g: macros.fat_g,
            carbs_g: macros.carbs_g,
            alcohol_g: macros.alcohol_g,
            timestamp,
        };

        log::append_entry(log_dir, date, &entry)?;

        writeln!(
            writer,
            "Added {} {} of {} to {}",
            servings,
            servings_word(servings),
            name,
            date
        )?;
        writeln!(writer)?;
        cmd_day(writer, log_dir, date, config)?;
        return Ok(());
    }

    let not_found = || {
        let ai_hint = if cfg!(feature = "ai") {
            ", or use `ai log` for AI-assisted logging"
        } else {
            ""
        };
        anyhow!(
            "no food '{}' found — log an ad-hoc entry by adding macro flags (e.g. `--calories 250`){ai_hint}",
            name
        )
    };
    let food_name = food::FoodName::from_str(name).map_err(|_| not_found())?;
    let path = food_name.file_path(foods_dir);
    if !path.exists() {
        return Err(not_found());
    }
    // Parse errors from an existing file keep their own message, which names
    // the file — only genuinely missing foods get the "add macro flags" hint.
    let food = food::load_food(&path)?;

    let ps = food.per_serving()?;

    let entry = log::LogEntry {
        title: food.title.clone(),
        servings,
        calories: ps.calories,
        protein_g: ps.protein_g,
        fiber_g: ps.fiber_g,
        fat_g: ps.fat_g,
        carbs_g: ps.carbs_g,
        alcohol_g: ps.alcohol_g,
        timestamp,
    };

    log::append_entry(log_dir, date, &entry)?;

    writeln!(
        writer,
        "Added {} {} of {} to {}",
        servings,
        servings_word(servings),
        food.title,
        date
    )?;
    writeln!(writer)?;
    cmd_day(writer, log_dir, date, config)?;

    Ok(())
}

pub(crate) fn cmd_day(
    writer: &mut impl Write,
    log_dir: &Path,
    date: chrono::NaiveDate,
    config: &Config,
) -> Result<()> {
    let day_log = log::load_day(log_dir, date)?;

    match day_log {
        None => writeln!(writer, "No entries for {}", date)?,
        Some(day_log) => write!(writer, "{}", render_day(&day_log, date, config)?)?,
    }

    Ok(())
}

/// The day-log table (plus the summary lines) for `day_log`, rendered to a
/// string. `date` only affects the "now" color scaling for today's rows.
pub(crate) fn render_day(
    day_log: &log::DayLog,
    date: chrono::NaiveDate,
    config: &Config,
) -> Result<String> {
    let columns = config.columns()?;
    let show_time = config.show_timestamp();
    let time_format = config.time_format();
    let mut headers: Vec<&str> = vec!["#", "Item", "Servings"];
    let mut aligns = vec![Align::Right, Align::Left, Align::Right];
    if show_time {
        headers.push("Time");
        aligns.push(Align::Right);
    }
    headers.extend(columns.iter().map(|c| c.label()));
    aligns.extend(columns.iter().map(|_| Align::Right));
    let mut table = Table::with_align(&headers, &aligns);
    table.set_title(&date.to_string());

    let rows = build_rows(&day_log.entries)?;

    let mut totals = Macros::ZERO;

    for (i, row) in rows.iter().enumerate() {
        let serv_str = display::servings_cell(row.servings.to_decimal());

        let mut cells = vec![(i + 1).to_string(), row.title.clone(), serv_str];
        if show_time {
            cells.push(time_cell(row.timestamp, time_format));
        }
        for column in &columns {
            cells.push(display::log_cell(*column, row.column_value(*column)));
        }
        table.add_row(cells);

        totals = totals
            .checked_add(&row.macros)
            .context("day macro total overflow")?;
    }

    let (net_cal, deficit) = log::day_net_and_deficit(
        totals.calories.to_decimal(),
        day_log.exercise_calories,
        config.maintenance_calories,
    )?;

    let now = Local::now();
    let now_time = (date == now.date_naive()).then(|| now.time());
    let targets = config.targets()?;
    let show_exercise =
        day_log.exercise_calories > Calories::ZERO && columns.contains(&Column::Calories);

    let mut plain_cells = Vec::new();
    let mut colored_cells = Vec::new();
    let mut exercise_cells = Vec::new();
    for column in &columns {
        let total = totals.column_value(*column);
        let colored = if *column == Column::Calories {
            net_cal
        } else {
            total
        };
        let color = display::column_color(now_time, colored, &targets.for_column(*column));
        plain_cells.push(display::log_cell(*column, total));
        colored_cells.push(display::wrap_color(
            &display::log_cell(*column, colored),
            color,
        ));
        if show_exercise {
            exercise_cells.push(if *column == Column::Calories {
                format!(
                    "-{}",
                    display::log_cell(Column::Calories, day_log.exercise_calories.to_decimal())
                )
            } else {
                String::new()
            });
        }
    }

    // The three leading cells plus one empty Time cell when the column is
    // shown; footers never carry time values.
    let meta_cells = if show_time { 4 } else { 3 };

    let mut total_row = vec![String::new(); meta_cells];
    total_row[1] = "Total".to_string();
    if show_exercise {
        total_row.extend(plain_cells);
        table.add_footer_custom(total_row);

        let mut exercise_row = vec![String::new(); meta_cells];
        exercise_row[1] = "Exercise".to_string();
        exercise_row.extend(exercise_cells);
        table.add_footer_custom(exercise_row);

        let mut net_row = vec![String::new(); meta_cells];
        net_row[1] = "Net".to_string();
        net_row.extend(colored_cells);
        table.add_footer_custom(net_row);
    } else {
        total_row.extend(colored_cells);
        table.add_footer_custom(total_row);
    }

    let mut out = table.format();

    let summary = display::render_day_summary(
        day_log.exercise_calories,
        config.maintenance_calories,
        deficit,
    )?;
    out.push_str(&summary);
    Ok(out)
}

struct DisplayRow {
    title: String,
    servings: Servings,
    macros: Macros,
    timestamp: Option<Timestamp>,
}

impl ColumnValue for DisplayRow {
    fn column_value(&self, column: Column) -> Decimal {
        self.macros.column_value(column)
    }
}

fn build_rows(entries: &[log::LogEntry]) -> Result<Vec<DisplayRow>> {
    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        rows.push(DisplayRow {
            title: entry.title.clone(),
            servings: entry.servings,
            macros: entry.totals()?,
            timestamp: entry.timestamp,
        });
    }
    Ok(rows)
}

/// The Time cell: the entry's timestamp in local time per the configured
/// format, or an empty cell when the entry has none.
fn time_cell(timestamp: Option<Timestamp>, format: TimeFormat) -> String {
    timestamp.map(|ts| ts.format(format)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::Grams;
    use crate::log::LogEntry;
    use std::str::FromStr;

    fn entry(title: &str, servings: &str) -> LogEntry {
        LogEntry {
            title: title.to_string(),
            servings: Servings::from_str(servings).unwrap(),
            calories: Calories::ZERO,
            protein_g: Grams::ZERO,
            fiber_g: Grams::ZERO,
            fat_g: Grams::ZERO,
            carbs_g: Grams::ZERO,
            alcohol_g: Grams::ZERO,
            timestamp: None,
        }
    }

    #[test]
    fn test_build_rows_one_per_entry() {
        let entries = vec![
            entry("coffee", "1.0"),
            entry("coffee", "2.0"),
            entry("oatmeal", "1.0"),
        ];
        let rows = build_rows(&entries).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn test_build_rows_titles_preserved() {
        let entries = vec![
            entry("Cherries - 155g", "1.0"),
            entry("Sour Cream - 60g", "1.0"),
        ];
        let rows = build_rows(&entries).unwrap();
        assert_eq!(rows[0].title, "Cherries - 155g");
        assert_eq!(rows[1].title, "Sour Cream - 60g");
    }

    #[test]
    fn test_build_rows_calories_scaled_by_servings() {
        let mut e = entry("coffee", "2.0");
        e.calories = Calories::from_str("24").unwrap();
        let rows = build_rows(&[e]).unwrap();
        assert_eq!(rows[0].macros.calories, Calories::from_str("48").unwrap());
    }

    #[test]
    fn test_build_rows_new_macros_scaled_by_servings() {
        let mut e = entry("coffee", "2.0");
        e.fat_g = Grams::from_str("5.0").unwrap();
        e.carbs_g = Grams::from_str("15.0").unwrap();
        e.alcohol_g = Grams::from_str("2.0").unwrap();
        let rows = build_rows(&[e]).unwrap();
        assert_eq!(rows[0].macros.fat_g, Grams::from_str("10.0").unwrap());
        assert_eq!(rows[0].macros.carbs_g, Grams::from_str("30.0").unwrap());
        assert_eq!(rows[0].macros.alcohol_g, Grams::from_str("4.0").unwrap());
    }

    #[test]
    fn test_build_rows_carries_timestamps() {
        let mut e = entry("coffee", "1.0");
        e.timestamp = Some(
            Timestamp::from_local(
                NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
                NaiveTime::from_hms_opt(12, 30, 0).unwrap(),
            )
            .unwrap(),
        );
        let rows = build_rows(&[e]).unwrap();
        assert!(rows[0].timestamp.is_some());

        let rows = build_rows(&[entry("coffee", "1.0")]).unwrap();
        assert!(rows[0].timestamp.is_none());
    }

    #[test]
    fn test_entry_timestamp_explicit_time_wins_over_toggle() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let time = NaiveTime::from_hms_opt(12, 30, 0).unwrap();

        let off: Config = toml::from_str("write_timestamps = false\n").unwrap();
        let ts = entry_timestamp(date, Some(time), &off).unwrap().unwrap();
        // The stamp is the given local time on the target date (verified by
        // display roundtrip at the same instant's local offset).
        assert_eq!(ts.format(TimeFormat::H24), "12:30");

        let on = Config::default();
        let ts = entry_timestamp(date, Some(time), &on).unwrap().unwrap();
        assert_eq!(ts.format(TimeFormat::H24), "12:30");
    }

    #[test]
    fn test_entry_timestamp_toggle_controls_default() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();

        let on = Config::default();
        assert!(entry_timestamp(date, None, &on).unwrap().is_some());

        let off: Config = toml::from_str("write_timestamps = false\n").unwrap();
        assert!(entry_timestamp(date, None, &off).unwrap().is_none());
    }

    #[test]
    fn test_time_cell_formats_and_empty() {
        let ts = Timestamp::from_local(
            NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            NaiveTime::from_hms_opt(14, 5, 0).unwrap(),
        )
        .unwrap();
        // Display roundtrip: any local zone renders the input time back.
        assert_eq!(time_cell(Some(ts), TimeFormat::H24), "14:05");
        assert_eq!(time_cell(None, TimeFormat::H24), "");
        assert_eq!(time_cell(None, TimeFormat::H12), "");
    }

    fn day_log(entries: Vec<LogEntry>) -> log::DayLog {
        log::DayLog {
            entries,
            exercise_calories: Calories::ZERO,
        }
    }

    #[test]
    fn test_render_day_shows_time_column_by_default() -> Result<()> {
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let mut e = entry("coffee", "1.0");
        e.calories = Calories::from_str("12").unwrap();
        let out = render_day(&day_log(vec![e]), date, &Config::default())?;
        assert!(out.contains("Time"), "time header missing: {out}");
        // The cell is the local-time rendering of the stamp (empty here).
        assert!(out.contains("Total"));
        Ok(())
    }

    #[test]
    fn test_render_day_time_column_optional() -> Result<()> {
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let mut e = entry("coffee", "1.0");
        e.calories = Calories::from_str("12").unwrap();
        let config: Config = toml::from_str("show_timestamp = false\n").unwrap();
        let out = render_day(&day_log(vec![e]), date, &config)?;
        assert!(!out.contains("Time"), "time column must be hidden: {out}");
        Ok(())
    }

    #[test]
    fn test_render_day_renders_real_time_cell() -> Result<()> {
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let mut e = entry("coffee", "1.0");
        e.calories = Calories::from_str("12").unwrap();
        e.timestamp = Some(Timestamp::from_local(
            date,
            NaiveTime::from_hms_opt(14, 5, 0).unwrap(),
        )?);
        let out = render_day(&day_log(vec![e]), date, &Config::default())?;
        // Display roundtrip: the local-time rendering of the stamp equals
        // the input time in any local timezone.
        assert!(out.contains("14:05"), "time cell missing: {out}");
        Ok(())
    }

    #[test]
    fn test_render_day_exercise_footer_with_time_column() -> Result<()> {
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let mut e = entry("coffee", "1.0");
        e.calories = Calories::from_str("12").unwrap();
        e.timestamp = Some(Timestamp::from_local(
            date,
            NaiveTime::from_hms_opt(14, 5, 0).unwrap(),
        )?);
        let mut day = day_log(vec![e]);
        day.exercise_calories = Calories::from_str("300").unwrap();
        let out = render_day(&day, date, &Config::default())?;
        assert!(out.contains("14:05"), "time cell missing: {out}");
        assert!(out.contains("Total"), "got: {out}");
        assert!(out.contains("Exercise"), "got: {out}");
        assert!(out.contains("Net"), "got: {out}");
        assert!(out.contains("-300"), "exercise subtraction missing: {out}");
        Ok(())
    }

    #[test]
    fn test_render_day_exercise_footer_without_time_column() -> Result<()> {
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let mut e = entry("coffee", "1.0");
        e.calories = Calories::from_str("12").unwrap();
        let mut day = day_log(vec![e]);
        day.exercise_calories = Calories::from_str("300").unwrap();
        let config: Config = toml::from_str("show_timestamp = false\n").unwrap();
        let out = render_day(&day, date, &config)?;
        assert!(!out.contains("Time"), "got: {out}");
        assert!(out.contains("Total"), "got: {out}");
        assert!(out.contains("Exercise"), "got: {out}");
        assert!(out.contains("Net"), "got: {out}");
        assert!(out.contains("-300"), "got: {out}");
        Ok(())
    }

    #[test]
    fn test_cmd_retime_sets_and_reports() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        log::append_entry(dir.path(), date, &entry("coffee", "1.0"))?;

        let mut out = Vec::new();
        let config = Config::default();
        cmd_retime(
            &mut out,
            dir.path(),
            date,
            1,
            NaiveTime::from_hms_opt(9, 15, 0).unwrap(),
            true,
            &config,
        )?;
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("Set entry 1"), "got: {text}");
        assert!(text.contains("to 09:15"), "got: {text}");

        let loaded = log::load_day(dir.path(), date)?.expect("day should exist");
        let ts = loaded.entries[0].timestamp.expect("timestamp must be set");
        assert_eq!(ts.format(config.time_format()), "09:15");
        Ok(())
    }

    #[test]
    fn test_cmd_retime_missing_entry_errors() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        log::append_entry(dir.path(), date, &entry("coffee", "1.0"))?;

        let mut out = Vec::new();
        let err = cmd_retime(
            &mut out,
            dir.path(),
            date,
            2,
            NaiveTime::from_hms_opt(9, 15, 0).unwrap(),
            true,
            &Config::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("entry 2 not found"));
        Ok(())
    }

    #[test]
    fn test_cmd_retime_no_day_errors() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let mut out = Vec::new();
        let err = cmd_retime(
            &mut out,
            dir.path(),
            date,
            1,
            NaiveTime::from_hms_opt(9, 15, 0).unwrap(),
            true,
            &Config::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("no entries"));
        Ok(())
    }
}
