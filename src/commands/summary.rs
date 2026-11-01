use crate::amount::{Calories, Macros};
use crate::config::{Column, Config};
use crate::display;
use crate::display::{ColumnValue, Table};
use crate::log;
use anyhow::{Context, Result};
use rust_decimal::Decimal;
use std::io::Write;
use std::path::Path;

struct SummaryRow {
    date: chrono::NaiveDate,
    macros: Macros,
    exercise_calories: Calories,
    deficit: Option<Decimal>,
    complete: bool,
}

impl ColumnValue for SummaryRow {
    fn column_value(&self, column: Column) -> Decimal {
        self.macros.column_value(column)
    }
}

fn build_summary_rows(
    log_dir: &Path,
    end: chrono::NaiveDate,
    days: u32,
    maintenance_calories: Option<Calories>,
) -> Result<Vec<SummaryRow>> {
    if !log_dir.is_dir() {
        return Ok(Vec::new());
    }

    let days = days.max(1);
    let start = end
        .checked_sub_days(chrono::Days::new((days - 1) as u64))
        .context("days range exceeds the supported date span")?;

    let mut dates: Vec<chrono::NaiveDate> = log::list_log_dates(log_dir)?
        .into_iter()
        .filter_map(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .filter(|d| *d >= start && *d <= end)
        .collect();
    dates.sort_unstable();

    let mut rows = Vec::with_capacity(dates.len());
    for date in dates {
        if let Some(day_log) = log::load_day(log_dir, date)? {
            let mut macros = Macros::ZERO;
            for entry in &day_log.entries {
                macros = macros
                    .checked_add(&entry.totals()?)
                    .context("day macro total overflow")?;
            }

            let (_, deficit) = log::day_net_and_deficit(
                macros.calories.to_decimal(),
                day_log.exercise_calories,
                maintenance_calories,
            )?;
            let complete = day_log.complete;

            rows.push(SummaryRow {
                date,
                macros,
                exercise_calories: day_log.exercise_calories,
                // A partial day's recorded intake is a lower bound, so its
                // "deficit" would be an upper bound at best. Leave it blank
                // and out of the period totals rather than show a number that
                // looks authoritative.
                deficit: if complete { deficit } else { None },
                complete,
            });
        }
    }
    Ok(rows)
}

pub(crate) fn cmd_summary(
    writer: &mut impl Write,
    log_dir: &Path,
    end: chrono::NaiveDate,
    days: u32,
    config: &Config,
) -> Result<()> {
    let rows = build_summary_rows(log_dir, end, days, config.maintenance_calories)?;

    if rows.is_empty() {
        let window = days.max(1);
        writeln!(
            writer,
            "No entries in the last {} day{} (ending {})",
            window,
            if window == 1 { "" } else { "s" },
            end
        )?;
        return Ok(());
    }

    let window = days.max(1);
    let complete_rows: Vec<&SummaryRow> = rows.iter().filter(|r| r.complete).collect();
    let complete_count = complete_rows.len();
    let partial_count = rows.len() - complete_count;
    let unlogged_count = (window as usize).saturating_sub(rows.len());

    let columns = config.columns()?;
    let any_exercise = rows.iter().any(|r| r.exercise_calories > Calories::ZERO)
        && columns.contains(&Column::Calories);
    let show_deficit = config.maintenance_calories.is_some();

    let mut headers: Vec<&str> = vec!["Date", "Status"];
    headers.extend(columns.iter().map(|c| c.label()));
    if any_exercise {
        headers.push("Exercise");
    }
    if show_deficit {
        headers.push("Deficit");
    }

    let mut table = Table::new(&headers);
    table.set_title(&format!(
        "Summary {} to {}",
        rows.first().expect("rows checked non-empty").date,
        rows.last().expect("rows checked non-empty").date
    ));

    for row in &rows {
        let mut cells = vec![
            row.date.to_string(),
            if row.complete {
                "complete".to_string()
            } else {
                "partial".to_string()
            },
        ];
        for column in &columns {
            cells.push(display::log_cell(*column, row.column_value(*column)));
        }
        if any_exercise {
            if row.exercise_calories > Calories::ZERO {
                cells.push(display::wrap_color(
                    &display::log_cell(Column::Calories, row.exercise_calories.to_decimal()),
                    Some(display::ANSI_BOLD_RED),
                ));
            } else {
                cells.push("0".to_string());
            }
        }
        if show_deficit {
            cells.push(match row.deficit {
                Some(d) => display::log_cell(Column::Calories, d),
                None => String::new(),
            });
        }
        table.add_row(cells);
    }

    if complete_count > 0 {
        let count = Decimal::from(complete_count);
        let mut totals = Macros::ZERO;
        for row in &complete_rows {
            totals = totals
                .checked_add(&row.macros)
                .context("period macro total overflow")?;
        }
        let total_exercise: Decimal = complete_rows.iter().try_fold(Decimal::ZERO, |acc, r| {
            acc.checked_add(r.exercise_calories.to_decimal())
                .context("exercise total overflow")
        })?;
        let total_deficit: Decimal =
            complete_rows
                .iter()
                .try_fold(Decimal::ZERO, |acc, r| match r.deficit {
                    Some(d) => acc.checked_add(d).context("period deficit overflow"),
                    None => Ok(acc),
                })?;

        // The Status column gets an empty cell in both footer rows.
        let mut total_footer = vec!["Total".to_string(), String::new()];
        for column in &columns {
            total_footer.push(display::log_cell(*column, totals.column_value(*column)));
        }
        if any_exercise {
            total_footer.push(display::log_cell(Column::Calories, total_exercise));
        }
        if show_deficit {
            total_footer.push(display::log_cell(Column::Calories, total_deficit));
        }
        table.add_footer(total_footer);

        let mut avg_footer = vec!["Avg/day".to_string(), String::new()];
        for column in &columns {
            avg_footer.push(display::log_cell(
                *column,
                totals
                    .column_value(*column)
                    .checked_div(count)
                    .expect("complete rows checked non-empty"),
            ));
        }
        if any_exercise {
            avg_footer.push(display::log_cell(
                Column::Calories,
                total_exercise
                    .checked_div(count)
                    .expect("complete rows checked non-empty"),
            ));
        }
        if show_deficit {
            avg_footer.push(display::log_cell(
                Column::Calories,
                total_deficit
                    .checked_div(count)
                    .expect("complete rows checked non-empty"),
            ));
        }
        table.add_footer(avg_footer);
    }

    write!(writer, "{}", table.format())?;

    writeln!(writer)?;
    if complete_count == 0 {
        writeln!(
            writer,
            "No complete days; totals and averages skipped ({partial_count} partial, {unlogged_count} not logged)"
        )?;
    } else {
        writeln!(
            writer,
            "{complete_count} of {window} days complete; totals and averages cover complete days only ({partial_count} partial, {unlogged_count} not logged)"
        )?;
    }

    if !show_deficit {
        writeln!(writer, "Set maintenance_calories in config to see deficit.")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::{Grams, Servings};
    use crate::log::LogEntry;
    use std::str::FromStr;

    fn write_day_log(
        dir: &Path,
        date: chrono::NaiveDate,
        calories: &str,
        protein: &str,
        exercise: u32,
        complete: bool,
    ) {
        let day_log = log::DayLog {
            entries: vec![LogEntry {
                title: "coffee".to_string(),
                servings: Servings::from_str("1.0").unwrap(),
                calories: Calories::from_str(calories).unwrap(),
                protein_g: Grams::from_str(protein).unwrap(),
                fiber_g: Grams::from_str("4.0").unwrap(),
                fat_g: Grams::from_str("2.0").unwrap(),
                carbs_g: Grams::from_str("8.0").unwrap(),
                alcohol_g: Grams::ZERO,
                timestamp: None,
            }],
            exercise_calories: Calories::from_str(&exercise.to_string()).unwrap(),
            weights: Vec::new(),
            complete,
        };
        let content = toml::to_string(&day_log).unwrap();
        std::fs::write(dir.join(format!("{}.toml", date)), content).unwrap();
    }

    #[test]
    fn test_build_summary_rows_skips_empty_days() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "100.0", "10.0", 0, true);
        write_day_log(
            dir.path(),
            end - chrono::Days::new(2),
            "200.0",
            "10.0",
            0,
            true,
        );

        let rows = build_summary_rows(dir.path(), end, 7, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, end - chrono::Days::new(2));
        assert_eq!(rows[1].date, end);
    }

    #[test]
    fn test_build_summary_rows_deficit_matches_day_math() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "1500.0", "10.0", 300, true);

        let rows = build_summary_rows(
            dir.path(),
            end,
            7,
            Some(Calories::from_str("2400").unwrap()),
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        // net = 1500 - 300 = 1200; deficit = maintenance - net = 2400 - 1200 = 1200
        assert_eq!(rows[0].deficit, Some(Decimal::from(1200)));
    }

    #[test]
    fn test_build_summary_rows_empty_range() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let rows = build_summary_rows(dir.path(), end, 7, None).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_build_summary_rows_days_zero_clamped() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "100.0", "10.0", 0, true);
        let rows = build_summary_rows(dir.path(), end, 0, None).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn test_build_summary_rows_days_overflow_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "100.0", "10.0", 0, true);
        let result = build_summary_rows(dir.path(), end, u32::MAX, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_summary_rows_ignores_non_date_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(
            dir.path(),
            end - chrono::Days::new(1),
            "100.0",
            "10.0",
            0,
            true,
        );
        std::fs::write(dir.path().join("README.toml"), "title = \"x\"\n").unwrap();
        let rows = build_summary_rows(dir.path(), end, 7, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, end - chrono::Days::new(1));
    }

    #[test]
    fn test_build_summary_rows_missing_log_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let rows = build_summary_rows(&dir.path().join("does-not-exist"), end, 7, None).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_build_summary_rows_partial_day_has_no_deficit() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "1500.0", "10.0", 0, true);
        write_day_log(
            dir.path(),
            end - chrono::Days::new(1),
            "500.0",
            "10.0",
            0,
            false,
        );

        let rows = build_summary_rows(
            dir.path(),
            end,
            7,
            Some(Calories::from_str("2400").unwrap()),
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].complete);
        assert_eq!(
            rows[0].deficit, None,
            "partial day must not report a deficit"
        );
        assert!(rows[1].complete);
        assert_eq!(rows[1].deficit, Some(Decimal::from(900)));
    }

    #[test]
    fn test_cmd_summary_excludes_partial_from_totals() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "1000.0", "10.0", 0, true);
        write_day_log(
            dir.path(),
            end - chrono::Days::new(1),
            "500.0",
            "10.0",
            0,
            false,
        );

        let config: Config =
            toml::from_str("max_calories = 2000\nmaintenance_calories = 2400\n").unwrap();
        let mut out = Vec::new();
        cmd_summary(&mut out, dir.path(), end, 2, &config).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(text.contains("Status"), "got: {text}");
        assert!(text.contains("partial"), "got: {text}");
        assert!(text.contains("1 of 2 days complete"), "got: {text}");

        let total_line = text
            .lines()
            .find(|l| l.contains("Total"))
            .expect("total footer missing");
        assert!(
            total_line.contains("1000"),
            "complete day total missing: {total_line}"
        );
        assert!(
            !total_line.contains("1500"),
            "partial day must be excluded from the total: {total_line}"
        );

        let avg_line = text
            .lines()
            .find(|l| l.contains("Avg/day"))
            .expect("average footer missing");
        assert!(
            avg_line.contains("1000"),
            "average must divide by complete days only: {avg_line}"
        );
    }

    #[test]
    fn test_cmd_summary_no_complete_days_skips_footers() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "500.0", "10.0", 0, false);

        let mut out = Vec::new();
        cmd_summary(&mut out, dir.path(), end, 7, &Config::default()).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(text.contains("No complete days"), "got: {text}");
        assert!(!text.contains("Total"), "no footers expected: {text}");
        assert!(!text.contains("Avg/day"), "no footers expected: {text}");
        assert!(text.contains("partial"), "row status missing: {text}");
    }
}
