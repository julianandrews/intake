use anyhow::{Context, Result};
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{CompleteEnv, CompletionCandidate, Shell};
use clap_complete::engine::ArgValueCandidates;
use chrono::Local;

const CLAP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Green.on_default());
use std::fs;
use std::path::{Path, PathBuf};

mod config;
mod log;
mod recipe;
mod search;

#[derive(Parser)]
#[command(name = "diet-tracker", color = clap::ColorChoice::Always, styles = CLAP_STYLES)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Directory containing recipe files
    #[arg(long)]
    foods_dir: Option<PathBuf>,

    /// Directory containing log files
    #[arg(long)]
    log_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a recipe to today's log
    Add {
        /// Recipe slug (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_recipes))]
        recipe: String,
        /// Number of servings (default: 1)
        #[arg(default_value = "1")]
        servings: f64,
    },
    /// Show today's totals
    Today,
    /// Show totals for a specific date (YYYY-MM-DD)
    Log {
        #[arg(add = ArgValueCandidates::new(complete_log_dates))]
        date: String,
    },
    /// Show a recipe with ingredients and per-serving values
    Show {
        /// Recipe slug (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_recipes))]
        recipe: String,
    },
    /// List all recipes with per-serving values
    List,
    /// Generate shell completion script
    Completions {
        /// Shell to generate completions for (bash, zsh, fish, powershell, elvish)
        shell: Shell,
        /// Install to the standard completion directory for the shell
        #[arg(long)]
        install: bool,
    },
    /// Add an ad-hoc entry with custom macros (no recipe file needed)
    Adhoc {
        /// Name of the item
        name: String,
        /// Number of servings (default: 1)
        servings: Option<f64>,
        /// Calories
        #[arg(long)]
        calories: u32,
        /// Protein in grams
        #[arg(long)]
        protein: f64,
        /// Fiber in grams
        #[arg(long)]
        fiber: f64,
    },
    /// Search for recipe combinations to fill remaining calories
    Fill {
        /// Maximum calories remaining
        #[arg(long)]
        max_cal: Option<u32>,
        /// Minimum protein needed (g)
        #[arg(long)]
        min_protein: Option<f64>,
        /// Minimum fiber needed (g)
        #[arg(long)]
        min_fiber: Option<f64>,
        /// Max results to show (show all if omitted)
        #[arg(long)]
        limit: Option<usize>,
        /// Exclude a recipe slug (repeatable)
        #[arg(long, add = ArgValueCandidates::new(complete_recipes))]
        exclude: Vec<String>,
        /// Require a recipe slug (repeatable)
        #[arg(long, add = ArgValueCandidates::new(complete_recipes))]
        include: Vec<String>,
        /// Max servings of any single recipe
        #[arg(long, default_value = "3")]
        max_servings: u32,
        /// Use daily goals from config minus today's log
        #[arg(long)]
        remaining: bool,
        /// Path to config file (default: alongside foods dir)
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

fn complete_recipes() -> Vec<CompletionCandidate> {
    let dir = std::env::var("DIET_FOODS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("foods")
        });
    match recipe::list_recipe_slugs(&dir) {
        Ok(slugs) => slugs.into_iter().map(CompletionCandidate::new).collect(),
        Err(e) => {
            eprintln!("warning: failed to list recipes for completion: {e}");
            Vec::new()
        }
    }
}

fn complete_log_dates() -> Vec<CompletionCandidate> {
    let dir = std::env::var("DIET_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("log")
        });
    match log::list_log_dates(&dir) {
        Ok(dates) => dates.into_iter().map(CompletionCandidate::new).collect(),
        Err(e) => {
            eprintln!("warning: failed to list log dates for completion: {e}");
            Vec::new()
        }
    }
}

fn completion_path(shell: &Shell) -> Result<PathBuf> {
    let (dir, filename) = match shell {
        Shell::Bash => {
            let base = std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                    PathBuf::from(home).join(".local").join("share")
                });
            (base.join("bash-completion").join("completions"), "diet-tracker".to_string())
        }
        Shell::Zsh => {
            let base = std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                    PathBuf::from(home).join(".local").join("share")
                });
            (base.join("zsh").join("completions"), "_diet-tracker".to_string())
        }
        Shell::Fish => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            (PathBuf::from(home).join(".config").join("fish").join("completions"), "diet-tracker.fish".to_string())
        }
        _ => anyhow::bail!("install not supported for {} shell", shell),
    };
    Ok(dir.join(filename))
}

fn main() -> Result<()> {
    let wrapper_path = {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("diet");
        if p.is_file() {
            Some(p)
        } else {
            None
        }
    };

    CompleteEnv::with_factory(Cli::command)
        .completer("diet-tracker")
        .complete();

    let cli = Cli::parse();

    let foods_dir = cli.foods_dir.unwrap_or_else(|| {
        std::env::var("DIET_FOODS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("../foods"))
    });

    let log_dir = cli.log_dir.unwrap_or_else(|| {
        std::env::var("DIET_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("../log"))
    });

    match cli.command {
        Commands::Add { recipe, servings } => {
            cmd_add(&foods_dir, &log_dir, &recipe, servings)?;
        }
        Commands::Today => {
            cmd_show(&foods_dir, &log_dir, Local::now().date_naive())?;
        }
        Commands::Log { date } => {
            let date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .context("date must be in YYYY-MM-DD format")?;
            cmd_show(&foods_dir, &log_dir, date)?;
        }
        Commands::Show { recipe } => {
            cmd_show_recipe(&foods_dir, &recipe)?;
        }
        Commands::List => {
            cmd_list(&foods_dir)?;
        }
        Commands::Completions { shell, install } => {
            if install {
                let path = completion_path(&shell)?;
                fs::create_dir_all(path.parent().context("completion path has no parent")?)?;
                let completer = wrapper_path.as_deref().unwrap_or(Path::new("diet-tracker"));
                let output = std::process::Command::new(completer)
                    .env("COMPLETE", shell.to_string())
                    .output()
                    .context("failed to generate completion script")?;
                if !output.status.success() {
                    anyhow::bail!("completion generation failed");
                }
                fs::write(&path, &output.stdout)
                    .with_context(|| format!("failed to write {}", path.display()))?;
                println!("Installed {} completions to {}", shell, path.display());
            } else {
                let mut cmd = Cli::command();
                clap_complete::generate(shell, &mut cmd, "diet-tracker", &mut std::io::stdout());
            }
        }
        Commands::Adhoc { name, servings, calories, protein, fiber } => {
            cmd_adhoc(&log_dir, &name, servings.unwrap_or(1.0), calories, protein, fiber)?;
        }
        Commands::Fill { max_cal, min_protein, min_fiber, limit, exclude, include, max_servings, remaining, config } => {
            cmd_fill(&foods_dir, &log_dir, max_cal, min_protein, min_fiber, limit, &exclude, &include, max_servings, remaining, config)?;
        }
    }

    Ok(())
}

fn cmd_adhoc(log_dir: &Path, name: &str, servings: f64, calories: u32, protein_g: f64, fiber_g: f64) -> Result<()> {
    let entry = log::LogEntry {
        slug: name.to_lowercase().replace(' ', "-"),
        hash: String::new(),
        servings,
        calories,
        protein_g,
        fiber_g,
        title: Some(name.to_string()),
    };

    let date = Local::now().date_naive();
    log::append_entry(log_dir, date, &entry)?;

    println!("Added {} serving(s) of {} to {}", servings, name, date);
    Ok(())
}

fn cmd_add(foods_dir: &Path, log_dir: &Path, slug: &str, servings: f64) -> Result<()> {
    let recipe_path = foods_dir.join(format!("{}.toml", slug));
    let recipe = recipe::load_recipe(&recipe_path)
        .with_context(|| format!("recipe '{}' not found", slug))?;

    let hash = recipe.hash().to_string();
    let ps = recipe.per_serving();

    let entry = log::LogEntry {
        slug: slug.to_string(),
        hash,
        servings,
        calories: ps.calories,
        protein_g: ps.protein_g,
        fiber_g: ps.fiber_g,
        title: None,
    };

    let date = Local::now().date_naive();
    log::append_entry(log_dir, date, &entry)?;

    println!("Added {} servings of {} to {}", servings, recipe.title, date);
    println!();
    cmd_show(foods_dir, log_dir, date)?;

    Ok(())
}

fn resolve_title(foods_dir: &Path, entry: &log::LogEntry) -> Result<String> {
    if let Some(title) = &entry.title {
        return Ok(title.clone());
    }
    let recipe_path = foods_dir.join(format!("{}.toml", entry.slug));
    let recipe = recipe::load_recipe(&recipe_path)
        .with_context(|| format!("recipe '{}' not found", entry.slug))?;
    Ok(recipe.title)
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";

fn cmd_show(foods_dir: &Path, log_dir: &Path, date: chrono::NaiveDate) -> Result<()> {
    let day_log = log::load_day(log_dir, date)?;

    match day_log {
        None => println!("No entries for {}", date),
        Some(day_log) => {
            struct Row {
                title: String,
                serv_str: String,
                cal_str: String,
                prot_str: String,
                fib_str: String,
            }

            let mut rows: Vec<Row> = Vec::new();
            let mut total_cal = 0.0;
            let mut total_protein = 0.0;
            let mut total_fiber = 0.0;

            for entry in &day_log.entries {
                let title = resolve_title(foods_dir, entry)?;
                let cal = entry.calories as f64 * entry.servings;
                let prot = entry.protein_g * entry.servings;
                let fib = entry.fiber_g * entry.servings;

                let serv_str = if entry.servings.fract() == 0.0 {
                    format!("{}", entry.servings as u32)
                } else {
                    format!("{:.1}", entry.servings)
                };

                rows.push(Row {
                    title,
                    serv_str,
                    cal_str: format!("{:.0}", cal),
                    prot_str: format!("{:.1}", prot),
                    fib_str: format!("{:.1}", fib),
                });

                total_cal += cal;
                total_protein += prot;
                total_fiber += fib;
            }

            let c_title = rows.iter().map(|r| r.title.len()).chain([5, 4]).max().unwrap();
            let c_serv = rows.iter().map(|r| r.serv_str.len()).chain([7, 8]).max().unwrap();
            let c_cal = rows.iter().map(|r| r.cal_str.len()).chain([3, 8]).max().unwrap();
            let c_prot = rows.iter().map(|r| r.prot_str.len()).chain([5, 10]).max().unwrap();
            let c_fib = rows.iter().map(|r| r.fib_str.len()).chain([4, 8]).max().unwrap();

            let sep_width = 2 + c_title + 1 + c_serv + 2 + c_cal + 2 + c_prot + 2 + c_fib;

            println!("{ANSI_BOLD_CYAN}{}{ANSI_RESET}", day_log.date);
            println!("{ANSI_CYAN}{}{ANSI_RESET}", "-".repeat(sep_width));

            println!(
                "  {ANSI_BOLD_YELLOW}{:<t$} {:>s$}  {:>c$}  {:>p$}  {:>f$}{ANSI_RESET}",
                "Item", "Servings", "Calories", "Protein(g)", "Fiber(g)",
                t = c_title, s = c_serv, c = c_cal, p = c_prot, f = c_fib
            );
            println!(
                "  {ANSI_CYAN}{:<t$} {:>s$}  {:>c$}  {:>p$}  {:>f$}{ANSI_RESET}",
                "-".repeat(c_title), "-".repeat(c_serv), "-".repeat(c_cal), "-".repeat(c_prot), "-".repeat(c_fib),
                t = c_title, s = c_serv, c = c_cal, p = c_prot, f = c_fib
            );

            for row in &rows {
                println!(
                    "  {:<t$} {:>s$}  {:>c$}  {:>p$}  {:>f$}",
                    row.title, row.serv_str, row.cal_str, row.prot_str, row.fib_str,
                    t = c_title, s = c_serv, c = c_cal, p = c_prot, f = c_fib
                );
            }

            println!("{ANSI_CYAN}{}{ANSI_RESET}", "-".repeat(sep_width));
            let total_cal_str = format!("{:.0}", total_cal);
            let total_prot_str = format!("{:.1}", total_protein);
            let total_fib_str = format!("{:.1}", total_fiber);
            println!(
                "  {ANSI_BOLD_GREEN}{:<t$} {:>s$}  {:>c$}  {:>p$}  {:>f$}{ANSI_RESET}",
                "TOTAL", "", total_cal_str, total_prot_str, total_fib_str,
                t = c_title, s = c_serv, c = c_cal, p = c_prot, f = c_fib
            );
        }
    }

    Ok(())
}

fn cmd_show_recipe(foods_dir: &Path, slug: &str) -> Result<()> {
    let recipe_path = foods_dir.join(format!("{}.toml", slug));
    let recipe = recipe::load_recipe(&recipe_path)
        .with_context(|| format!("recipe '{}' not found", slug))?;
    println!("{}", recipe.display());
    Ok(())
}

fn cmd_list(foods_dir: &Path) -> Result<()> {
    let recipes = recipe::find_all_recipes(foods_dir)?;

    println!("{:<24} {:>8} {:>8} {:>6} {:>5}", "Recipe", "Servings", "Cal/serv", "Prot", "Fiber");
    println!("{}", "-".repeat(55));

    for (_, recipe) in &recipes {
        let ps = recipe.per_serving();
        println!(
            "{:<24} {:>8} {:>8} {:>5.1}g {:>4.1}g",
            recipe.title,
            recipe.servings,
            ps.calories,
            ps.protein_g,
            ps.fiber_g,
        );
    }

    Ok(())
}

fn cmd_fill(
    foods_dir: &Path,
    log_dir: &Path,
    max_calories: Option<u32>,
    min_protein: Option<f64>,
    min_fiber: Option<f64>,
    limit: Option<usize>,
    exclude: &[String],
    include: &[String],
    max_servings: u32,
    remaining: bool,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let config_path = config_path.unwrap_or_else(|| {
        let base = foods_dir.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        base.join("diet-config.toml")
    });

    let cfg_result = config::load_config(&config_path);
    let search_cfg = cfg_result.as_ref().ok();
    let max_nodes = search_cfg.map(|c| c.search.max_nodes).unwrap_or(100_000);
    let max_results = search_cfg.map(|c| c.search.max_results).unwrap_or(1000);

    let (max_calories, min_protein, min_fiber) = if remaining {
        let cfg = cfg_result.context("config required for --remaining mode (provide --config or place diet-config.toml alongside foods dir)")?;
        let goals = &cfg.goals;

        let today = chrono::Local::now().date_naive();
        let day_log = log::load_day(log_dir, today)?;
        let (consumed_cal, consumed_prot, consumed_fib) = match day_log {
            Some(log) => {
                let cal: u32 = log.entries.iter().map(|e| (e.calories as f64 * e.servings).round() as u32).sum();
                let prot: f64 = log.entries.iter().map(|e| e.protein_g * e.servings).sum();
                let fib: f64 = log.entries.iter().map(|e| e.fiber_g * e.servings).sum();
                (cal, prot, fib)
            }
            None => (0, 0.0, 0.0),
        };

        (
            max_calories.unwrap_or_else(|| goals.max_calories.saturating_sub(consumed_cal)),
            min_protein.unwrap_or_else(|| (goals.min_protein - consumed_prot).max(0.0)),
            min_fiber.unwrap_or_else(|| (goals.min_fiber - consumed_fib).max(0.0)),
        )
    } else {
        (
            max_calories.context("--max-cal is required when not using --remaining")?,
            min_protein.context("--min-protein is required when not using --remaining")?,
            min_fiber.context("--min-fiber is required when not using --remaining")?,
        )
    };

    let config = search::FillConfig {
        max_calories,
        min_protein_g: min_protein,
        min_fiber_g: min_fiber,
        max_servings_per_recipe: max_servings,
        limit,
        exclude,
        include,
        foods_dir,
        max_nodes,
        max_results,
    };

    let results = search::find_fills(&config)?;

    if results.is_empty() {
        println!("No combinations found.");
        return Ok(());
    }

    println!(
        "Ways to fill {} cal (need ≥{}g protein, ≥{}g fiber):",
        max_calories, min_protein, min_fiber
    );
    println!();

    for (i, r) in results.iter().enumerate() {
        let items: Vec<String> = r
            .selections
            .iter()
            .map(|s| format!("{} ({})", s.title, s.servings))
            .collect();
        let combo = items.join(" + ");

        println!(" {}. {}", i + 1, combo);
        println!(
            "    {} cal | {:.1}g protein | {:.1}g fiber ✓",
            r.total_calories, r.total_protein_g, r.total_fiber_g
        );
    }

    Ok(())
}
