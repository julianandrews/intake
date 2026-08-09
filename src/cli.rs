use crate::amount::{Calories, Grams, Servings};
use crate::completion::{complete_foods, complete_log_dates};
use crate::food::Slug;
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Parser, Subcommand};
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
    pub(crate) command: Commands,

    /// Directory containing food files
    #[arg(long)]
    pub(crate) foods_dir: Option<PathBuf>,

    /// Directory containing log files
    #[arg(long)]
    pub(crate) log_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Add a food to today's log
    Add {
        /// Food slug (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_foods))]
        food: Slug,
        /// Number of servings (default: 1)
        #[arg(default_value = "1")]
        servings: Servings,
    },
    /// Show totals for a date (default: today)
    Log {
        #[arg(add = ArgValueCandidates::new(complete_log_dates))]
        date: Option<String>,
        /// Number of days before today to show (e.g. 1 = yesterday)
        #[arg(long, short = 'd', conflicts_with = "date")]
        days_ago: Option<u32>,
    },
    /// Show a food with ingredients and per-serving values
    Show {
        /// Food slug (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_foods))]
        food: Slug,
    },
    /// List all foods with per-serving values
    List,
    /// Generate shell completion script
    Completions {
        /// Shell to generate completions for (bash, zsh, fish, powershell, elvish)
        shell: Shell,
        /// Install to the standard completion directory for the shell
        #[arg(long)]
        install: bool,
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
    /// Add an ad-hoc entry with custom macros (no food file needed)
    Adhoc {
        /// Name of the item
        #[arg(value_parser = non_blank_name)]
        name: String,
        /// Number of servings (default: 1)
        servings: Option<Servings>,
        /// Calories
        #[arg(long)]
        calories: Option<Calories>,
        /// Protein in grams
        #[arg(long)]
        protein: Option<Grams>,
        /// Fiber in grams
        #[arg(long)]
        fiber: Option<Grams>,
        /// Fat in grams
        #[arg(long)]
        fat: Option<Grams>,
        /// Carbs in grams
        #[arg(long)]
        carbs: Option<Grams>,
        /// Alcohol in grams
        #[arg(long)]
        alcohol: Option<Grams>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_days_ago_short_flag_parses() {
        let cli = Cli::try_parse_from(["intake", "log", "-d", "2"]).unwrap();
        match cli.command {
            Commands::Log { date, days_ago } => {
                assert_eq!(date, None);
                assert_eq!(days_ago, Some(2));
            }
            _ => panic!("expected Log command"),
        }
    }

    #[test]
    fn test_log_date_and_days_ago_conflict() {
        assert!(Cli::try_parse_from(["intake", "log", "2026-08-01", "--days-ago", "2"]).is_err());
    }

    #[test]
    fn test_summary_days_short_flag_parses() {
        let cli = Cli::try_parse_from(["intake", "summary", "-d", "5"]).unwrap();
        match cli.command {
            Commands::Summary { date, days } => {
                assert_eq!(date, None);
                assert_eq!(days, 5);
            }
            _ => panic!("expected Summary command"),
        }
    }

    #[test]
    fn test_invalid_slug_rejected_at_parse() {
        assert!(Cli::try_parse_from(["intake", "add", ""]).is_err());
        assert!(Cli::try_parse_from(["intake", "add", "a/b"]).is_err());
        assert!(Cli::try_parse_from(["intake", "add", ".."]).is_err());
        assert!(Cli::try_parse_from(["intake", "show", "."]).is_err());
        assert!(Cli::try_parse_from(["intake", "add", "coffee"]).is_ok());
    }

    #[test]
    fn test_adhoc_name_rejects_blank_and_trims() {
        assert!(Cli::try_parse_from(["intake", "adhoc", ""]).is_err());
        assert!(Cli::try_parse_from(["intake", "adhoc", "   "]).is_err());
        let cli = Cli::try_parse_from(["intake", "adhoc", "  Greek yogurt  "]).unwrap();
        match cli.command {
            Commands::Adhoc { name, .. } => assert_eq!(name, "Greek yogurt"),
            _ => panic!("expected Adhoc command"),
        }
    }
}
