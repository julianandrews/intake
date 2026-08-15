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

fn parse_time(s: &str) -> Result<chrono::NaiveTime, String> {
    chrono::NaiveTime::parse_from_str(s, "%H:%M")
        .map_err(|_| "time must be in HH:MM 24-hour format".to_string())
}

const CLAP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Green.on_default());

#[derive(Parser)]
#[command(
    name = "intake",
    about = "Show today's log (use --date or --days-ago for other days)",
    color = clap::ColorChoice::Auto,
    styles = CLAP_STYLES
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,

    /// Directory containing food files
    #[arg(long)]
    pub(crate) foods_dir: Option<PathBuf>,

    /// Directory containing log files
    #[arg(long)]
    pub(crate) log_dir: Option<PathBuf>,

    /// The shared target-date argument: accepted by the day view and every
    /// date-targeting command (log, ai log, rm, exercise, summary).
    #[command(flatten)]
    pub(crate) date: DateArgs,
}

/// The shared target-date arguments: `--date` or `--days-ago` target a day
/// relative to today. Flattened into the root command (the day view) and
/// every date-targeting command; date args on a subcommand win over the
/// root's.
#[derive(Args, Clone, Default)]
pub(crate) struct DateArgs {
    /// Date to target (YYYY-MM-DD, default: today)
    #[arg(long, add = ArgValueCandidates::new(complete_log_dates))]
    pub(crate) date: Option<String>,
    /// Number of days before today to target (e.g. 1 = yesterday)
    #[arg(long, short = 'd', conflicts_with = "date")]
    pub(crate) days_ago: Option<u32>,
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
        /// Time of day to stamp the entry with (HH:MM, local) instead of now
        #[arg(long, value_name = "HH:MM", value_parser = parse_time)]
        time: Option<chrono::NaiveTime>,
        #[command(flatten)]
        date: DateArgs,
    },
    /// Show a multi-day summary of macros and deficit
    Summary {
        /// Number of days to look back (including the end date; default: config summary_days, or 7)
        #[arg(long)]
        days: Option<u32>,
        #[command(flatten)]
        date: DateArgs,
    },
    /// Record exercise calories for a day
    Exercise {
        /// Calories burned
        calories: Calories,
        #[command(flatten)]
        date: DateArgs,
    },
    /// Remove an entry from a day's log
    Rm {
        /// Entry number to remove (see the # column in the day view, `intake`)
        #[arg(value_parser = clap::value_parser!(u32).range(1..))]
        index: u32,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        date: DateArgs,
    },
    /// Set an entry's timestamp
    Retime {
        /// Entry number to re-time (see the # column in the day view, `intake`)
        #[arg(value_parser = clap::value_parser!(u32).range(1..))]
        index: u32,
        /// New time of day (HH:MM, local)
        #[arg(value_name = "HH:MM", value_parser = parse_time)]
        time: chrono::NaiveTime,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        date: DateArgs,
    },
    /// Manage foods
    Food {
        #[command(subcommand)]
        command: FoodCommands,
    },
    /// AI-assisted commands (prompt a model to log or edit foods)
    #[cfg(feature = "ai")]
    Ai {
        #[command(subcommand)]
        command: crate::ai::cli::AiCommands,
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
                time,
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
                assert_eq!(time, None);
                assert_eq!(date.date, None);
                assert_eq!(date.days_ago, None);
            }
            _ => panic!("expected Log command"),
        }
    }

    #[test]
    fn test_log_time_flag_parses() {
        let cli = Cli::try_parse_from(["intake", "log", "coffee", "--time", "14:30"]).unwrap();
        match cli.command {
            Some(Commands::Log { time, date, .. }) => {
                assert_eq!(
                    time,
                    Some(chrono::NaiveTime::from_hms_opt(14, 30, 0).unwrap())
                );
                assert_eq!(date.date, None);
            }
            _ => panic!("expected Log command"),
        }

        // Composes with the date flags.
        let cli =
            Cli::try_parse_from(["intake", "log", "coffee", "-d", "1", "--time", "08:05"]).unwrap();
        match cli.command {
            Some(Commands::Log { time, date, .. }) => {
                assert_eq!(
                    time,
                    Some(chrono::NaiveTime::from_hms_opt(8, 5, 0).unwrap())
                );
                assert_eq!(date.days_ago, Some(1));
            }
            _ => panic!("expected Log command"),
        }
    }

    #[test]
    fn test_log_time_rejects_bad_format() {
        assert!(Cli::try_parse_from(["intake", "log", "coffee", "--time", "3pm"]).is_err());
        assert!(Cli::try_parse_from(["intake", "log", "coffee", "--time", "14:30:45"]).is_err());
        assert!(Cli::try_parse_from(["intake", "log", "coffee", "--time", "25:00"]).is_err());
    }

    #[test]
    fn test_retime_parses_index_time_yes_and_date() {
        let cli = Cli::try_parse_from(["intake", "retime", "2", "14:30"]).unwrap();
        match cli.command {
            Some(Commands::Retime {
                index,
                time,
                yes,
                date,
            }) => {
                assert_eq!(index, 2);
                assert_eq!(time, chrono::NaiveTime::from_hms_opt(14, 30, 0).unwrap());
                assert!(!yes);
                assert_eq!(date.date, None);
            }
            _ => panic!("expected Retime command"),
        }

        let cli = Cli::try_parse_from([
            "intake",
            "retime",
            "3",
            "08:05",
            "--yes",
            "--date",
            "2026-08-01",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Retime {
                index,
                time,
                yes,
                date,
            }) => {
                assert_eq!(index, 3);
                assert_eq!(time, chrono::NaiveTime::from_hms_opt(8, 5, 0).unwrap());
                assert!(yes);
                assert_eq!(date.date, Some("2026-08-01".to_string()));
            }
            _ => panic!("expected Retime command"),
        }
    }

    #[test]
    fn test_retime_rejects_zero_index_and_bad_time() {
        assert!(Cli::try_parse_from(["intake", "retime", "0", "14:30"]).is_err());
        assert!(Cli::try_parse_from(["intake", "retime", "1", "half past"]).is_err());
        assert!(Cli::try_parse_from(["intake", "retime", "1"]).is_err());
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
                assert_eq!(date.days_ago, None);
            }
            _ => panic!("expected Log command"),
        }
        assert_eq!(cli.date.date, None);
    }

    #[test]
    fn test_log_days_ago_short_flag_parses() {
        let cli = Cli::try_parse_from(["intake", "log", "coffee", "-d", "1"]).unwrap();
        match cli.command {
            Some(Commands::Log { date, .. }) => {
                assert_eq!(date.date, None);
                assert_eq!(date.days_ago, Some(1));
            }
            _ => panic!("expected Log command"),
        }
        assert_eq!(cli.date.days_ago, None);
    }

    #[test]
    fn test_root_and_subcommand_date_args_parse_separately() {
        let cli = Cli::try_parse_from(["intake", "-d", "1", "log", "coffee", "-d", "2"]).unwrap();
        assert_eq!(cli.date.days_ago, Some(1));
        match cli.command {
            Some(Commands::Log { date, .. }) => assert_eq!(date.days_ago, Some(2)),
            _ => panic!("expected Log command"),
        }
    }

    #[test]
    fn test_food_rejects_date_args() {
        assert!(Cli::try_parse_from(["intake", "food", "list", "--days-ago", "1"]).is_err());
        assert!(Cli::try_parse_from(["intake", "food", "list", "--date", "2026-08-01"]).is_err());
        assert!(Cli::try_parse_from(["intake", "completions", "bash", "-d", "1"]).is_err());
    }

    #[test]
    fn test_date_and_days_ago_conflict() {
        assert!(
            Cli::try_parse_from(["intake", "--date", "2026-08-01", "--days-ago", "2"]).is_err()
        );
        assert!(Cli::try_parse_from([
            "intake",
            "log",
            "coffee",
            "--date",
            "2026-08-01",
            "--days-ago",
            "2"
        ])
        .is_err());
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
    fn test_bare_intake_days_ago_parses() {
        let cli = Cli::try_parse_from(["intake", "-d", "2"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.date.date, None);
        assert_eq!(cli.date.days_ago, Some(2));
    }

    #[test]
    fn test_summary_parses_days() {
        let cli = Cli::try_parse_from(["intake", "summary", "--days", "5"]).unwrap();
        match cli.command {
            Some(Commands::Summary { days, date }) => {
                assert_eq!(days, Some(5));
                assert_eq!(date.date, None);
                assert_eq!(date.days_ago, None);
            }
            _ => panic!("expected Summary command"),
        }
    }

    #[test]
    fn test_summary_days_short_flag_is_days_ago() {
        let cli = Cli::try_parse_from(["intake", "summary", "-d", "5"]).unwrap();
        match cli.command {
            Some(Commands::Summary { days, date }) => {
                assert_eq!(days, None);
                assert_eq!(date.days_ago, Some(5));
            }
            _ => panic!("expected Summary command"),
        }
        assert_eq!(cli.date.days_ago, None);
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

        let cli = Cli::try_parse_from(["intake", "rm", "3", "--yes", "--days-ago", "1"]).unwrap();
        match cli.command {
            Some(Commands::Rm { date, .. }) => assert_eq!(date.days_ago, Some(1)),
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
