use crate::cli::LogDateArgs;
use crate::completion::complete_foods;
use crate::food::FoodName;
use clap::{Args, Subcommand};
use clap_complete::engine::ArgValueCandidates;

#[derive(Subcommand)]
pub(crate) enum AiCommands {
    /// Edit a day's log with AI
    Log {
        /// Prompt for the edit (default: open $EDITOR)
        prompt: Vec<String>,
        #[command(flatten)]
        date: LogDateArgs,
        #[command(flatten)]
        flags: AiFlags,
    },
    /// AI-powered food commands
    Food {
        #[command(subcommand)]
        command: AiFoodCommands,
    },
}

#[derive(Subcommand)]
pub(crate) enum AiFoodCommands {
    /// Create a new food with AI
    New {
        /// Food name (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_foods))]
        name: FoodName,
        /// Prompt for the recipe (default: open $EDITOR)
        prompt: Vec<String>,
        #[command(flatten)]
        flags: AiFlags,
    },
    /// Edit a food with AI
    Edit {
        /// Food name (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_foods))]
        name: FoodName,
        /// Prompt for the edit (default: open $EDITOR)
        prompt: Vec<String>,
        #[command(flatten)]
        flags: AiFlags,
    },
}

#[derive(Args)]
pub(crate) struct AiFlags {
    /// Override the API key
    #[arg(long)]
    pub(crate) api_key: Option<String>,
    /// Override the model
    #[arg(long)]
    pub(crate) model: Option<String>,
    /// Override the base URL of the OpenAI-compatible endpoint
    #[arg(long)]
    pub(crate) base_url: Option<String>,
    /// Skip confirmation and accept the proposal
    #[arg(long)]
    pub(crate) yes: bool,
    /// Print the request messages sent to the model to stderr
    #[arg(long)]
    pub(crate) trace_requests: bool,
    /// Print the model's responses (reasoning, output, parse errors) to stderr
    #[arg(long)]
    pub(crate) trace_responses: bool,
    /// Provide the prompt inline instead of opening the editor
    #[arg(long = "prompt", value_name = "PROMPT", conflicts_with = "prompt")]
    pub(crate) prompt_arg: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn test_bare_ai_is_usage_error() {
        assert!(Cli::try_parse_from(["intake", "ai"]).is_err());
    }

    #[test]
    fn test_ai_log_parses_prompt_and_date() {
        let cli = Cli::try_parse_from([
            "intake",
            "ai",
            "log",
            "add",
            "dinner",
            "--date",
            "2026-08-01",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Ai {
                command:
                    AiCommands::Log {
                        prompt,
                        date,
                        flags,
                    },
            }) => {
                assert_eq!(prompt, vec!["add", "dinner"]);
                assert_eq!(date.date.as_deref(), Some("2026-08-01"));
                assert!(!flags.yes);
                assert!(!flags.trace_requests);
                assert!(!flags.trace_responses);
            }
            _ => panic!("expected Ai Log command"),
        }
    }

    #[test]
    fn test_ai_log_date_has_no_short_flag() {
        assert!(Cli::try_parse_from(["intake", "ai", "log", "-d", "1"]).is_err());
    }

    #[test]
    fn test_ai_log_no_prompt_allowed() {
        let cli = Cli::try_parse_from(["intake", "ai", "log"]).unwrap();
        match cli.command {
            Some(Commands::Ai {
                command: AiCommands::Log { prompt, .. },
            }) => assert!(prompt.is_empty()),
            _ => panic!("expected Ai Log command"),
        }
    }

    #[test]
    fn test_ai_log_shared_flags() {
        let cli = Cli::try_parse_from([
            "intake",
            "ai",
            "log",
            "--api-key",
            "k",
            "--model",
            "m",
            "--base-url",
            "http://localhost:11434/v1",
            "--yes",
            "--trace-requests",
            "--trace-responses",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Ai {
                command: AiCommands::Log { flags, .. },
            }) => {
                assert_eq!(flags.api_key.as_deref(), Some("k"));
                assert_eq!(flags.model.as_deref(), Some("m"));
                assert_eq!(flags.base_url.as_deref(), Some("http://localhost:11434/v1"));
                assert!(flags.yes);
                assert!(flags.trace_requests);
                assert!(flags.trace_responses);
            }
            _ => panic!("expected Ai Log command"),
        }
    }

    #[test]
    fn test_ai_log_trace_flags_independent() {
        let cli = Cli::try_parse_from(["intake", "ai", "log", "--trace-requests"]).unwrap();
        match cli.command {
            Some(Commands::Ai {
                command: AiCommands::Log { flags, .. },
            }) => {
                assert!(flags.trace_requests);
                assert!(!flags.trace_responses);
            }
            _ => panic!("expected Ai Log command"),
        }
        let cli = Cli::try_parse_from(["intake", "ai", "log", "--trace-responses"]).unwrap();
        match cli.command {
            Some(Commands::Ai {
                command: AiCommands::Log { flags, .. },
            }) => {
                assert!(!flags.trace_requests);
                assert!(flags.trace_responses);
            }
            _ => panic!("expected Ai Log command"),
        }
    }

    #[test]
    fn test_ai_log_prompt_flag_inline() {
        let cli = Cli::try_parse_from(["intake", "ai", "log", "--prompt", "add dinner"]).unwrap();
        match cli.command {
            Some(Commands::Ai {
                command: AiCommands::Log { prompt, flags, .. },
            }) => {
                assert!(prompt.is_empty());
                assert_eq!(flags.prompt_arg.as_deref(), Some("add dinner"));
            }
            _ => panic!("expected Ai Log command"),
        }
    }

    #[test]
    fn test_ai_log_prompt_flag_conflicts_with_positional() {
        assert!(Cli::try_parse_from(["intake", "ai", "log", "add", "--prompt", "x"]).is_err());
    }

    #[test]
    fn test_ai_food_new_parses_name_and_prompt() {
        let cli =
            Cli::try_parse_from(["intake", "ai", "food", "new", "my-food", "recipe"]).unwrap();
        match cli.command {
            Some(Commands::Ai {
                command:
                    AiCommands::Food {
                        command: AiFoodCommands::New { name, prompt, .. },
                    },
            }) => {
                assert_eq!(name.to_string(), "my-food");
                assert_eq!(prompt, vec!["recipe"]);
            }
            _ => panic!("expected Ai Food New command"),
        }
    }

    #[test]
    fn test_ai_food_edit_parses_name_and_yes() {
        let cli = Cli::try_parse_from(["intake", "ai", "food", "edit", "coffee", "--yes"]).unwrap();
        match cli.command {
            Some(Commands::Ai {
                command:
                    AiCommands::Food {
                        command: AiFoodCommands::Edit { name, flags, .. },
                    },
            }) => {
                assert_eq!(name.to_string(), "coffee");
                assert!(flags.yes);
            }
            _ => panic!("expected Ai Food Edit command"),
        }
    }

    #[test]
    fn test_ai_food_invalid_name_rejected() {
        assert!(Cli::try_parse_from(["intake", "ai", "food", "new", "a/b"]).is_err());
        assert!(Cli::try_parse_from(["intake", "ai", "food", "new", ""]).is_err());
    }

    #[test]
    fn test_ai_requires_subcommand() {
        assert!(Cli::try_parse_from(["intake", "ai", "food"]).is_err());
    }
}
