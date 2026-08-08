use crate::amount::{Calories, Grams, Macros, Servings};
use crate::cli::{Cli, Commands};
use crate::completion;
use crate::config::Config;
use anyhow::{Context, Result};
use chrono::Local;
use clap::CommandFactory;

mod food;
mod log;
mod summary;

pub(crate) fn run(command: Commands, config: &Config) -> Result<()> {
    let foods_dir = config.foods_dir();
    let log_dir = config.log_dir();
    let mut stdout = std::io::stdout();

    match command {
        Commands::Add { food, servings } => {
            log::cmd_add(&mut stdout, &foods_dir, &log_dir, &food, servings, config)?;
        }
        Commands::Log { date, days_ago } => {
            let date = resolve_date(date, days_ago)?;
            log::cmd_log(&mut stdout, &log_dir, date, config)?;
        }
        Commands::Show { food } => {
            food::cmd_show_food(&mut stdout, &foods_dir, &food, config)?;
        }
        Commands::List => {
            food::cmd_list(&mut stdout, &foods_dir, config)?;
        }
        Commands::Summary { date, days } => {
            let end = resolve_date(date, None)?;
            summary::cmd_summary(&mut stdout, &log_dir, end, days, config)?;
        }
        Commands::Exercise { calories } => {
            log::cmd_exercise(&mut stdout, &log_dir, calories)?;
        }
        Commands::Completions { shell, install } => {
            completion::cmd_completions(&mut stdout, shell, install, Cli::command())?;
        }
        Commands::Adhoc {
            name,
            servings,
            calories,
            protein,
            fiber,
            fat,
            carbs,
            alcohol,
        } => {
            log::cmd_adhoc(
                &mut stdout,
                &log_dir,
                &name,
                servings.unwrap_or(Servings::from_u32(1)),
                &Macros {
                    calories: calories.unwrap_or(Calories::ZERO),
                    protein_g: protein.unwrap_or(Grams::ZERO),
                    fiber_g: fiber.unwrap_or(Grams::ZERO),
                    fat_g: fat.unwrap_or(Grams::ZERO),
                    carbs_g: carbs.unwrap_or(Grams::ZERO),
                    alcohol_g: alcohol.unwrap_or(Grams::ZERO),
                },
            )?;
        }
    }

    Ok(())
}

fn resolve_date(date: Option<String>, days_ago: Option<u32>) -> Result<chrono::NaiveDate> {
    if let Some(d) = date {
        return chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
            .context("date must be in YYYY-MM-DD format");
    }
    let today = Local::now().date_naive();
    match days_ago {
        Some(n) => today
            .checked_sub_days(chrono::Days::new(n as u64))
            .context("days_ago exceeds the supported date span"),
        None => Ok(today),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_date_defaults_to_today() {
        let today = Local::now().date_naive();
        let date = resolve_date(None, None).unwrap();
        assert_eq!(date, today);
    }

    #[test]
    fn test_resolve_date_days_ago_zero_is_today() {
        let today = Local::now().date_naive();
        let date = resolve_date(None, Some(0)).unwrap();
        assert_eq!(date, today);
    }

    #[test]
    fn test_resolve_date_days_ago_one_is_yesterday() {
        let today = Local::now().date_naive();
        let date = resolve_date(None, Some(1)).unwrap();
        assert_eq!(date, today - chrono::Days::new(1));
    }

    #[test]
    fn test_resolve_date_positional_wins() {
        let date = resolve_date(Some("2026-08-01".to_string()), Some(3)).unwrap();
        assert_eq!(date, chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
    }

    #[test]
    fn test_resolve_date_invalid_format_errors() {
        assert!(resolve_date(Some("yesterday".to_string()), None).is_err());
    }

    #[test]
    fn test_resolve_date_overflow_errors() {
        assert!(resolve_date(None, Some(u32::MAX)).is_err());
    }
}
