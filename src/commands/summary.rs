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

            rows.push(SummaryRow {
                date,
                macros,
                exercise_calories: day_log.exercise_calories,
                deficit,
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

    let columns = config.columns()?;
    let any_exercise = rows.iter().any(|r| r.exercise_calories > Calories::ZERO)
        && columns.contains(&Column::Calories);
    let show_deficit = config.maintenance_calories.is_some();

    let mut headers: Vec<&str> = vec!["Date"];
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
        let mut cells = vec![row.date.to_string()];
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
        if let Some(d) = row.deficit {
            cells.push(display::log_cell(Column::Calories, d));
        }
        table.add_row(cells);
    }

    let count = Decimal::from(rows.len());
    let mut totals = Macros::ZERO;
    for row in &rows {
        totals = totals
            .checked_add(&row.macros)
            .context("period macro total overflow")?;
    }
    let total_exercise: Decimal = rows.iter().try_fold(Decimal::ZERO, |acc, r| {
        acc.checked_add(r.exercise_calories.to_decimal())
            .context("exercise total overflow")
    })?;
    let total_deficit: Decimal = rows
        .iter()
        .try_fold(Decimal::ZERO, |acc, r| match r.deficit {
            Some(d) => acc.checked_add(d).context("period deficit overflow"),
            None => Ok(acc),
        })?;

    let mut total_footer = vec!["Total".to_string()];
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

    let mut avg_footer = vec!["Avg/day".to_string()];
    for column in &columns {
        avg_footer.push(display::log_cell(
            *column,
            totals
                .column_value(*column)
                .checked_div(count)
                .expect("rows checked non-empty"),
        ));
    }
    if any_exercise {
        avg_footer.push(display::log_cell(
            Column::Calories,
            total_exercise
                .checked_div(count)
                .expect("rows checked non-empty"),
        ));
    }
    if show_deficit {
        avg_footer.push(display::log_cell(
            Column::Calories,
            total_deficit
                .checked_div(count)
                .expect("rows checked non-empty"),
        ));
    }
    table.add_footer(avg_footer);

    write!(writer, "{}", table.format())?;

    if !show_deficit {
        writeln!(writer)?;
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
        };
        let content = toml::to_string(&day_log).unwrap();
        std::fs::write(dir.join(format!("{}.toml", date)), content).unwrap();
    }

    #[test]
    fn test_build_summary_rows_skips_empty_days() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "100.0", "10.0", 0);
        write_day_log(dir.path(), end - chrono::Days::new(2), "200.0", "10.0", 0);

        let rows = build_summary_rows(dir.path(), end, 7, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, end - chrono::Days::new(2));
        assert_eq!(rows[1].date, end);
    }

    #[test]
    fn test_build_summary_rows_deficit_matches_day_math() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "1500.0", "10.0", 300);

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
        write_day_log(dir.path(), end, "100.0", "10.0", 0);
        let rows = build_summary_rows(dir.path(), end, 0, None).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn test_build_summary_rows_days_overflow_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, "100.0", "10.0", 0);
        let result = build_summary_rows(dir.path(), end, u32::MAX, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_summary_rows_ignores_non_date_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end - chrono::Days::new(1), "100.0", "10.0", 0);
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
}
