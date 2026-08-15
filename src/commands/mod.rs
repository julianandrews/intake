use crate::amount::{Calories, Grams, Macros};
use crate::cli::{Cli, Commands, DateArgs, FoodCommands};
use crate::completion;
use crate::config::Config;
use anyhow::{bail, Context, Result};
use chrono::Local;
use clap::CommandFactory;

pub(crate) mod food;
pub(crate) mod log;
mod summary;

pub(crate) fn run(command: Option<Commands>, root_date: DateArgs, config: &Config) -> Result<()> {
    let foods_dir = config.foods_dir();
    let log_dir = config.log_dir();
    let mut stdout = std::io::stdout();

    match command {
        None => {
            let date = resolve_date(root_date.date, root_date.days_ago)?;
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
            time,
            date,
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
            let merged = merged_date(&root_date, &date);
            let date = resolve_date(merged.date, merged.days_ago)?;
            log::cmd_log(
                &mut stdout,
                &foods_dir,
                &log_dir,
                log::LogRequest {
                    name: &name,
                    servings,
                    macros: &macros,
                    adhoc,
                    time,
                },
                date,
                config,
            )?;
        }
        Some(Commands::Summary { days, date }) => {
            let merged = merged_date(&root_date, &date);
            let end = resolve_date(merged.date, merged.days_ago)?;
            let days = days.unwrap_or_else(|| config.summary_days());
            summary::cmd_summary(&mut stdout, &log_dir, end, days, config)?;
        }
        Some(Commands::Exercise { calories, date }) => {
            let merged = merged_date(&root_date, &date);
            let date = resolve_date(merged.date, merged.days_ago)?;
            log::cmd_exercise(&mut stdout, &log_dir, date, calories)?;
        }
        Some(Commands::Rm { index, yes, date }) => {
            let merged = merged_date(&root_date, &date);
            let date = resolve_date(merged.date, merged.days_ago)?;
            log::cmd_rm(&mut stdout, &log_dir, date, index, yes, config)?;
        }
        Some(Commands::Retime {
            index,
            time,
            yes,
            date,
        }) => {
            let merged = merged_date(&root_date, &date);
            let date = resolve_date(merged.date, merged.days_ago)?;
            log::cmd_retime(&mut stdout, &log_dir, date, index, time, yes, config)?;
        }
        Some(Commands::Food { command }) => {
            reject_root_date(&root_date, "food")?;
            match command {
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
                FoodCommands::Rm { name, yes } => {
                    food::cmd_rm_food(&mut stdout, &foods_dir, &name, yes)?;
                }
            }
        }
        #[cfg(feature = "ai")]
        Some(Commands::Ai { command }) => match command {
            crate::ai::cli::AiCommands::Log {
                prompt,
                date,
                flags,
            } => {
                let merged = merged_date(&root_date, &date);
                crate::ai::run(
                    &mut stdout,
                    &foods_dir,
                    &log_dir,
                    crate::ai::cli::AiCommands::Log {
                        prompt,
                        date: merged,
                        flags,
                    },
                    config,
                )?;
            }
            crate::ai::cli::AiCommands::Food { .. } => {
                reject_root_date(&root_date, "ai food")?;
                crate::ai::run(&mut stdout, &foods_dir, &log_dir, command, config)?;
            }
        },
        Some(Commands::Completions { shell, install }) => {
            reject_root_date(&root_date, "completions")?;
            completion::cmd_completions(&mut stdout, shell, install, Cli::command())?;
        }
    }

    Ok(())
}

/// Merge the root-level and subcommand-level date args: date args on a
/// subcommand win wholesale; otherwise the root's apply (the day view, or
/// as a fallback for date-targeting commands).
fn merged_date(root: &DateArgs, cmd: &DateArgs) -> DateArgs {
    if cmd.date.is_some() || cmd.days_ago.is_some() {
        cmd.clone()
    } else {
        root.clone()
    }
}

/// Date args on the bare command target the day view or a date-targeting
/// command; on `name` (which targets neither) they are an error.
fn reject_root_date(root: &DateArgs, name: &str) -> Result<()> {
    if root.date.is_some() || root.days_ago.is_some() {
        bail!(
            "--date/--days-ago target the day view (bare `intake`) or a date-targeting \
             command (log, summary, exercise, rm, retime, ai log); `{name}` doesn't take them"
        );
    }
    Ok(())
}

fn parse_date(date: &str) -> Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").context("date must be in YYYY-MM-DD format")
}

pub(crate) fn resolve_date(
    date: Option<String>,
    days_ago: Option<u32>,
) -> Result<chrono::NaiveDate> {
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
    fn test_resolve_date_date_wins_over_days_ago() {
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
    fn test_merged_date_root_only() {
        let root = DateArgs {
            date: None,
            days_ago: Some(2),
        };
        let cmd = DateArgs::default();
        let merged = merged_date(&root, &cmd);
        assert_eq!(merged.date, None);
        assert_eq!(merged.days_ago, Some(2));
    }

    #[test]
    fn test_merged_date_subcommand_only() {
        let root = DateArgs::default();
        let cmd = DateArgs {
            date: Some("2026-08-01".to_string()),
            days_ago: None,
        };
        let merged = merged_date(&root, &cmd);
        assert_eq!(merged.date, Some("2026-08-01".to_string()));
        assert_eq!(merged.days_ago, None);
    }

    #[test]
    fn test_merged_date_subcommand_wins() {
        let root = DateArgs {
            date: None,
            days_ago: Some(1),
        };
        let cmd = DateArgs {
            date: None,
            days_ago: Some(2),
        };
        let merged = merged_date(&root, &cmd);
        assert_eq!(merged.date, None);
        assert_eq!(merged.days_ago, Some(2));
    }

    #[test]
    fn test_merged_date_subcommand_wins_wholesale() {
        let root = DateArgs {
            date: Some("2026-08-01".to_string()),
            days_ago: None,
        };
        let cmd = DateArgs {
            date: None,
            days_ago: Some(2),
        };
        let merged = merged_date(&root, &cmd);
        assert_eq!(merged.date, None);
        assert_eq!(merged.days_ago, Some(2));
    }

    #[test]
    fn test_reject_root_date_errors_when_set() {
        let root = DateArgs {
            date: None,
            days_ago: Some(1),
        };
        assert!(reject_root_date(&root, "food").is_err());
        let root = DateArgs {
            date: Some("2026-08-01".to_string()),
            days_ago: None,
        };
        assert!(reject_root_date(&root, "food").is_err());
    }

    #[test]
    fn test_reject_root_date_ok_when_unset() {
        let root = DateArgs::default();
        assert!(reject_root_date(&root, "food").is_ok());
    }
}
