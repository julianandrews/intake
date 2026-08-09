use crate::amount::{Calories, Macros, Servings};
use crate::config::{Column, Config};
use crate::display;
use crate::display::{ColumnValue, Table};
use crate::{food, log};
use anyhow::{Context, Result};
use chrono::Local;
use rust_decimal::Decimal;
use std::io::Write;
use std::path::Path;

pub(crate) fn cmd_exercise(
    writer: &mut impl Write,
    log_dir: &Path,
    calories: Calories,
) -> Result<()> {
    let date = Local::now().date_naive();
    log::set_exercise_calories(log_dir, date, calories)?;
    writeln!(
        writer,
        "Recorded {} exercise calories for {}",
        calories, date
    )?;
    Ok(())
}

pub(crate) fn cmd_adhoc(
    writer: &mut impl Write,
    log_dir: &Path,
    name: &str,
    servings: Servings,
    macros: &Macros,
) -> Result<()> {
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

    let date = Local::now().date_naive();
    log::append_entry(log_dir, date, &entry)?;

    writeln!(
        writer,
        "Added {} serving(s) of {} to {}",
        servings, name, date
    )?;
    Ok(())
}

pub(crate) fn cmd_add(
    writer: &mut impl Write,
    foods_dir: &Path,
    log_dir: &Path,
    slug: &food::Slug,
    servings: Servings,
    config: &Config,
) -> Result<()> {
    let food = food::load_food(&slug.file_path(foods_dir))
        .with_context(|| format!("food '{}' not found", slug))?;

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

    let date = Local::now().date_naive();
    log::append_entry(log_dir, date, &entry)?;

    writeln!(
        writer,
        "Added {} servings of {} to {}",
        servings, food.title, date
    )?;
    writeln!(writer)?;
    cmd_log(writer, log_dir, date, config)?;

    Ok(())
}

pub(crate) fn cmd_log(
    writer: &mut impl Write,
    log_dir: &Path,
    date: chrono::NaiveDate,
    config: &Config,
) -> Result<()> {
    let day_log = log::load_day(log_dir, date)?;

    match day_log {
        None => writeln!(writer, "No entries for {}", date)?,
        Some(day_log) => {
            let columns = config.columns()?;
            let mut headers: Vec<&str> = vec!["Item", "Servings"];
            headers.extend(columns.iter().map(|c| c.label()));
            let mut table = Table::new(&headers);
            table.set_title(&date.to_string());

            let rows = build_rows(&day_log.entries)?;

            let mut totals = Macros::ZERO;
            let mut total_servings = Decimal::ZERO;

            for row in &rows {
                let serv_str = display::servings_cell(row.servings.to_decimal());

                let mut cells = vec![row.title.clone(), serv_str];
                for column in &columns {
                    cells.push(display::log_cell(*column, row.column_value(*column)));
                }
                table.add_row(cells);

                totals = totals
                    .checked_add(&row.macros)
                    .context("day macro total overflow")?;
                total_servings = total_servings
                    .checked_add(row.servings.to_decimal())
                    .context("day servings total overflow")?;
            }

            let (net_cal, deficit) = log::day_net_and_deficit(
                totals.calories.to_decimal(),
                day_log.exercise_calories,
                config.maintenance_calories,
            )?;

            let now = (date == Local::now().date_naive()).then(|| Local::now().time());
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
                let color = display::column_color(now, colored, &targets.for_column(*column));
                plain_cells.push(display::log_cell(*column, total));
                colored_cells.push(display::wrap_color(
                    &display::log_cell(*column, colored),
                    color,
                ));
                if show_exercise {
                    exercise_cells.push(if *column == Column::Calories {
                        format!(
                            "-{}",
                            display::log_cell(
                                Column::Calories,
                                day_log.exercise_calories.to_decimal()
                            )
                        )
                    } else {
                        String::new()
                    });
                }
            }

            let mut total_row = vec!["Total".to_string(), display::servings_cell(total_servings)];
            if show_exercise {
                total_row.extend(plain_cells);
                table.add_footer_custom(total_row);

                let mut exercise_row = vec!["Exercise".to_string(), String::new()];
                exercise_row.extend(exercise_cells);
                table.add_footer_custom(exercise_row);

                let mut net_row = vec!["Net".to_string(), String::new()];
                net_row.extend(colored_cells);
                table.add_footer_custom(net_row);
            } else {
                total_row.extend(colored_cells);
                table.add_footer_custom(total_row);
            }

            write!(writer, "{}", table.format())?;

            let summary = display::render_day_summary(
                day_log.exercise_calories,
                config.maintenance_calories,
                deficit,
            )?;
            write!(writer, "{}", summary)?;
        }
    }

    Ok(())
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
        e.calories = Calories::from_u32(24);
        let rows = build_rows(&[e]).unwrap();
        assert_eq!(rows[0].macros.calories, Calories::from_u32(48));
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
