use crate::amount::{Calories, Macros, Servings};
use crate::config::{Column, Config};
use crate::confirm;
use crate::display;
use crate::display::{Align, ColumnValue, Table};
use crate::{food, log};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Local;
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
/// `macros` supplies the ad-hoc values.
pub(crate) struct LogRequest<'a> {
    pub name: &'a str,
    pub servings: Servings,
    pub macros: &'a Macros,
    pub adhoc: bool,
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
    } = request;
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
    let mut headers: Vec<&str> = vec!["#", "Item", "Servings"];
    headers.extend(columns.iter().map(|c| c.label()));
    let mut aligns = vec![Align::Right, Align::Left, Align::Right];
    aligns.extend(columns.iter().map(|_| Align::Right));
    let mut table = Table::with_align(&headers, &aligns);
    table.set_title(&date.to_string());

    let rows = build_rows(&day_log.entries)?;

    let mut totals = Macros::ZERO;

    for (i, row) in rows.iter().enumerate() {
        let serv_str = display::servings_cell(row.servings.to_decimal());

        let mut cells = vec![(i + 1).to_string(), row.title.clone(), serv_str];
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

    let mut total_row = vec![String::new(), "Total".to_string(), String::new()];
    if show_exercise {
        total_row.extend(plain_cells);
        table.add_footer_custom(total_row);

        let mut exercise_row = vec![String::new(), "Exercise".to_string(), String::new()];
        exercise_row.extend(exercise_cells);
        table.add_footer_custom(exercise_row);

        let mut net_row = vec![String::new(), "Net".to_string(), String::new()];
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
        });
    }
    Ok(rows)
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
}
