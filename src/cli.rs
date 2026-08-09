use crate::amount::{Calories, Grams, Servings};
use crate::completion::{complete_foods, complete_log_dates};
use crate::food::FoodName;
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Args, Parser, Subcommand};
use clap_complete::engine::ArgValueCandidates;
use clap_complete::Shell;
use std::path::PathBuf;

fn non_blank_name(s: &str) -> Result<String, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("name must not be blank".to_string());
    }
    Ok(trimmed.to_string())
}

const CLAP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Green.on_default());

#[derive(Parser)]
#[command(name = "intake", color = clap::ColorChoice::Auto, styles = CLAP_STYLES)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,

    /// Directory containing food files
    #[arg(long)]
    pub(crate) foods_dir: Option<PathBuf>,

    /// Directory containing log files
    #[arg(long)]
    pub(crate) log_dir: Option<PathBuf>,
}

/// The shared log-date argument: logging commands target a day via `--date`.
#[derive(Args)]
pub(crate) struct LogDateArgs {
    /// Date to log to (YYYY-MM-DD, default: today)
    #[arg(long, add = ArgValueCandidates::new(complete_log_dates))]
    pub(crate) date: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Log a food, or an ad-hoc entry when macro flags are given
    Log {
        /// Food name (filename without .toml), or the item's name for an ad-hoc entry
        #[arg(value_parser = non_blank_name, add = ArgValueCandidates::new(complete_foods))]
        name: String,
        /// Number of servings (default: 1)
        #[arg(default_value = "1")]
        servings: Servings,
        /// Calories (ad-hoc entry)
        #[arg(long)]
        calories: Option<Calories>,
        /// Protein in grams (ad-hoc entry)
        #[arg(long)]
        protein: Option<Grams>,
        /// Fiber in grams (ad-hoc entry)
        #[arg(long)]
        fiber: Option<Grams>,
        /// Fat in grams (ad-hoc entry)
        #[arg(long)]
        fat: Option<Grams>,
        /// Carbs in grams (ad-hoc entry)
        #[arg(long)]
        carbs: Option<Grams>,
        /// Alcohol in grams (ad-hoc entry)
        #[arg(long)]
        alcohol: Option<Grams>,
        #[command(flatten)]
        date: LogDateArgs,
    },
    /// Show a day's totals (default: today)
    Day {
        #[arg(add = ArgValueCandidates::new(complete_log_dates))]
        date: Option<String>,
        /// Number of days before today to show (e.g. 1 = yesterday)
        #[arg(long, short = 'd', conflicts_with = "date")]
        days_ago: Option<u32>,
    },
    /// Show a multi-day summary of macros and deficit
    Summary {
        /// End date (default: today)
        #[arg(add = ArgValueCandidates::new(complete_log_dates))]
        date: Option<String>,
        /// Number of days to look back (including the end date)
        #[arg(long, short = 'd', default_value = "7")]
        days: u32,
    },
    /// Record exercise calories for today
    Exercise {
        /// Calories burned
        calories: Calories,
    },
    /// Remove an entry from a day's log
    Rm {
        /// Entry number to remove (see the # column in `intake day`)
        #[arg(value_parser = clap::value_parser!(u32).range(1..))]
        index: u32,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        date: LogDateArgs,
    },
    /// Manage foods
    Food {
        #[command(subcommand)]
        command: FoodCommands,
    },
    /// Generate shell completion script
    Completions {
        /// Shell to generate completions for (bash, zsh, fish, powershell, elvish)
        shell: Shell,
        /// Install to the standard completion directory for the shell
        #[arg(long)]
        install: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum FoodCommands {
    /// List all foods with per-serving values
    List,
    /// Show a food with ingredients and per-serving values
    Show {
        /// Food name (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_foods))]
        food: FoodName,
    },
    /// Create a new food in the editor
    New {
        /// Food name (filename without .toml)
        name: FoodName,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Edit an existing food in the editor
    Edit {
        /// Food name (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_foods))]
        name: FoodName,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Delete a food file (existing log entries are unaffected)
    Rm {
        /// Food name (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_foods))]
        name: FoodName,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_bare_intake_parses() {
        let cli = Cli::try_parse_from(["intake"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_log_name_servings_and_macros() {
        let cli = Cli::try_parse_from([
            "intake",
            "log",
            "coffee",
            "2",
            "--calories",
            "100",
            "--protein",
            "5.5",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Log {
                name,
                servings,
                calories,
                protein,
                fiber,
                fat,
                carbs,
                alcohol,
                date,
            }) => {
                assert_eq!(name, "coffee");
                assert_eq!(servings, Servings::from_str("2").unwrap());
                assert_eq!(calories, Some(Calories::from_str("100").unwrap()));
                assert_eq!(protein, Some(Grams::from_str("5.5").unwrap()));
                assert_eq!(fiber, None);
                assert_eq!(fat, None);
                assert_eq!(carbs, None);
                assert_eq!(alcohol, None);
                assert_eq!(date.date, None);
            }
            _ => panic!("expected Log command"),
        }
    }

    #[test]
    fn test_log_servings_defaults_to_one() {
        let cli = Cli::try_parse_from(["intake", "log", "coffee"]).unwrap();
        match cli.command {
            Some(Commands::Log { servings, .. }) => assert_eq!(servings, Servings::ONE),
            _ => panic!("expected Log command"),
        }
    }

    #[test]
    fn test_log_date_flag_parses() {
        let cli = Cli::try_parse_from(["intake", "log", "coffee", "--date", "2026-08-01"]).unwrap();
        match cli.command {
            Some(Commands::Log { date, .. }) => {
                assert_eq!(date.date, Some("2026-08-01".to_string()));
            }
            _ => panic!("expected Log command"),
        }
    }

    #[test]
    fn test_log_date_has_no_short_flag() {
        assert!(Cli::try_parse_from(["intake", "log", "coffee", "-d", "1"]).is_err());
    }

    #[test]
    fn test_log_name_rejects_blank_and_trims() {
        assert!(Cli::try_parse_from(["intake", "log", ""]).is_err());
        assert!(Cli::try_parse_from(["intake", "log", "   "]).is_err());
        let cli = Cli::try_parse_from(["intake", "log", "  Greek yogurt  "]).unwrap();
        match cli.command {
            Some(Commands::Log { name, .. }) => assert_eq!(name, "Greek yogurt"),
            _ => panic!("expected Log command"),
        }
    }

    #[test]
    fn test_day_days_ago_short_flag_parses() {
        let cli = Cli::try_parse_from(["intake", "day", "-d", "2"]).unwrap();
        match cli.command {
            Some(Commands::Day { date, days_ago }) => {
                assert_eq!(date, None);
                assert_eq!(days_ago, Some(2));
            }
            _ => panic!("expected Day command"),
        }
    }

    #[test]
    fn test_day_date_and_days_ago_conflict() {
        assert!(Cli::try_parse_from(["intake", "day", "2026-08-01", "--days-ago", "2"]).is_err());
    }

    #[test]
    fn test_summary_days_short_flag_parses() {
        let cli = Cli::try_parse_from(["intake", "summary", "-d", "5"]).unwrap();
        match cli.command {
            Some(Commands::Summary { date, days }) => {
                assert_eq!(date, None);
                assert_eq!(days, 5);
            }
            _ => panic!("expected Summary command"),
        }
    }

    #[test]
    fn test_rm_parses_index_yes_and_date() {
        let cli = Cli::try_parse_from(["intake", "rm", "2"]).unwrap();
        match cli.command {
            Some(Commands::Rm { index, yes, date }) => {
                assert_eq!(index, 2);
                assert!(!yes);
                assert_eq!(date.date, None);
            }
            _ => panic!("expected Rm command"),
        }

        let cli =
            Cli::try_parse_from(["intake", "rm", "3", "--yes", "--date", "2026-08-01"]).unwrap();
        match cli.command {
            Some(Commands::Rm { index, yes, date }) => {
                assert_eq!(index, 3);
                assert!(yes);
                assert_eq!(date.date, Some("2026-08-01".to_string()));
            }
            _ => panic!("expected Rm command"),
        }
    }

    #[test]
    fn test_rm_rejects_zero_index() {
        assert!(Cli::try_parse_from(["intake", "rm", "0"]).is_err());
    }

    #[test]
    fn test_food_group_subcommands() {
        let cli = Cli::try_parse_from(["intake", "food", "list"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Food { .. })));

        let cli = Cli::try_parse_from(["intake", "food", "show", "coffee"]).unwrap();
        match cli.command {
            Some(Commands::Food {
                command: FoodCommands::Show { food },
            }) => assert_eq!(food.to_string(), "coffee"),
            _ => panic!("expected Food Show command"),
        }
    }

    #[test]
    fn test_food_new_parses_name_and_yes() {
        let cli = Cli::try_parse_from(["intake", "food", "new", "my-food"]).unwrap();
        match cli.command {
            Some(Commands::Food {
                command: FoodCommands::New { name, yes },
            }) => {
                assert_eq!(name.to_string(), "my-food");
                assert!(!yes);
            }
            _ => panic!("expected Food New command"),
        }

        let cli = Cli::try_parse_from(["intake", "food", "new", "my-food", "--yes"]).unwrap();
        match cli.command {
            Some(Commands::Food {
                command: FoodCommands::New { yes, .. },
            }) => assert!(yes),
            _ => panic!("expected Food New command"),
        }
    }

    #[test]
    fn test_food_edit_parses_name_and_yes() {
        let cli = Cli::try_parse_from(["intake", "food", "edit", "coffee", "--yes"]).unwrap();
        match cli.command {
            Some(Commands::Food {
                command: FoodCommands::Edit { name, yes },
            }) => {
                assert_eq!(name.to_string(), "coffee");
                assert!(yes);
            }
            _ => panic!("expected Food Edit command"),
        }
    }

    #[test]
    fn test_food_rm_parses_name_and_yes() {
        let cli = Cli::try_parse_from(["intake", "food", "rm", "coffee"]).unwrap();
        match cli.command {
            Some(Commands::Food {
                command: FoodCommands::Rm { name, yes },
            }) => {
                assert_eq!(name.to_string(), "coffee");
                assert!(!yes);
            }
            _ => panic!("expected Food Rm command"),
        }

        let cli = Cli::try_parse_from(["intake", "food", "rm", "coffee", "--yes"]).unwrap();
        match cli.command {
            Some(Commands::Food {
                command: FoodCommands::Rm { yes, .. },
            }) => assert!(yes),
            _ => panic!("expected Food Rm command"),
        }
    }

    #[test]
    fn test_invalid_name_rejected_at_parse() {
        assert!(Cli::try_parse_from(["intake", "food", "new", ""]).is_err());
        assert!(Cli::try_parse_from(["intake", "food", "new", "a/b"]).is_err());
        assert!(Cli::try_parse_from(["intake", "food", "new", ".."]).is_err());
        assert!(Cli::try_parse_from(["intake", "food", "show", "."]).is_err());
        assert!(Cli::try_parse_from(["intake", "food", "new", "coffee"]).is_ok());
    }
}
