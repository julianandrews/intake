use crate::amount::{Calories, Grams, Macros};
use crate::cli::{Cli, Commands, FoodCommands};
use crate::completion;
use crate::config::Config;
use anyhow::{Context, Result};
use chrono::Local;
use clap::CommandFactory;

mod food;
mod log;
mod summary;

pub(crate) fn run(command: Option<Commands>, config: &Config) -> Result<()> {
    let foods_dir = config.foods_dir();
    let log_dir = config.log_dir();
    let mut stdout = std::io::stdout();

    match command {
        None => {
            let date = Local::now().date_naive();
            log::cmd_day(&mut stdout, &log_dir, date, config)?;
        }
        Some(Commands::Log {
            name,
            servings,
            calories,
            protein,
            fiber,
            fat,
            carbs,
            alcohol,
            date: date_args,
        }) => {
            let adhoc = calories.is_some()
                || protein.is_some()
                || fiber.is_some()
                || fat.is_some()
                || carbs.is_some()
                || alcohol.is_some();
            let macros = Macros {
                calories: calories.unwrap_or(Calories::ZERO),
                protein_g: protein.unwrap_or(Grams::ZERO),
                fiber_g: fiber.unwrap_or(Grams::ZERO),
                fat_g: fat.unwrap_or(Grams::ZERO),
                carbs_g: carbs.unwrap_or(Grams::ZERO),
                alcohol_g: alcohol.unwrap_or(Grams::ZERO),
            };
            let date = resolve_log_date(date_args.date)?;
            log::cmd_log(
                &mut stdout,
                &foods_dir,
                &log_dir,
                log::LogRequest {
                    name: &name,
                    servings,
                    macros: &macros,
                    adhoc,
                },
                date,
                config,
            )?;
        }
        Some(Commands::Day { date, days_ago }) => {
            let date = resolve_date(date, days_ago)?;
            log::cmd_day(&mut stdout, &log_dir, date, config)?;
        }
        Some(Commands::Summary { date, days }) => {
            let end = resolve_date(date, None)?;
            summary::cmd_summary(&mut stdout, &log_dir, end, days, config)?;
        }
        Some(Commands::Exercise { calories }) => {
            log::cmd_exercise(&mut stdout, &log_dir, calories)?;
        }
        Some(Commands::Food { command }) => match command {
            FoodCommands::List => {
                food::cmd_list(&mut stdout, &foods_dir, config)?;
            }
            FoodCommands::Show { food } => {
                food::cmd_show_food(&mut stdout, &foods_dir, &food, config)?;
            }
            FoodCommands::New { name, yes } => {
                food::cmd_new_food(&mut stdout, &foods_dir, &name, yes, config)?;
            }
            FoodCommands::Edit { name, yes } => {
                food::cmd_edit_food(&mut stdout, &foods_dir, &name, yes, config)?;
            }
        },
        Some(Commands::Completions { shell, install }) => {
            completion::cmd_completions(&mut stdout, shell, install, Cli::command())?;
        }
    }

    Ok(())
}

fn parse_date(date: &str) -> Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").context("date must be in YYYY-MM-DD format")
}

fn resolve_date(date: Option<String>, days_ago: Option<u32>) -> Result<chrono::NaiveDate> {
    if let Some(d) = date {
        return parse_date(&d);
    }
    let today = Local::now().date_naive();
    match days_ago {
        Some(n) => today
            .checked_sub_days(chrono::Days::new(n as u64))
            .context("days_ago exceeds the supported date span"),
        None => Ok(today),
    }
}

fn resolve_log_date(date: Option<String>) -> Result<chrono::NaiveDate> {
    match date {
        Some(d) => parse_date(&d),
        None => Ok(Local::now().date_naive()),
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

    #[test]
    fn test_resolve_log_date_defaults_to_today() {
        let today = Local::now().date_naive();
        let date = resolve_log_date(None).unwrap();
        assert_eq!(date, today);
    }

    #[test]
    fn test_resolve_log_date_parses() {
        let date = resolve_log_date(Some("2026-08-01".to_string())).unwrap();
        assert_eq!(date, chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
    }

    #[test]
    fn test_resolve_log_date_invalid_format_errors() {
        assert!(resolve_log_date(Some("yesterday".to_string())).is_err());
    }
}
