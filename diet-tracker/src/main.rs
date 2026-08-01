use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use chrono::Local;
use std::path::{Path, PathBuf};

mod config;
mod log;
mod recipe;
mod search;

#[derive(Parser)]
#[command(name = "diet")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Directory containing recipe files
    #[arg(long, default_value = "../foods")]
    foods_dir: PathBuf,

    /// Directory containing log files
    #[arg(long, default_value = "../log")]
    log_dir: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a recipe to today's log
    Add {
        /// Recipe slug (filename without .toml)
        recipe: String,
        /// Number of servings
        servings: f64,
    },
    /// Show today's totals
    Today,
    /// Show totals for a specific date (YYYY-MM-DD)
    Log {
        date: String,
    },
    /// Show a recipe with ingredients and per-serving values
    Show {
        /// Recipe slug (filename without .toml)
        recipe: String,
    },
    /// List all recipes with per-serving values
    List,
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
        #[arg(long)]
        exclude: Vec<String>,
        /// Require a recipe slug (repeatable)
        #[arg(long)]
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { recipe, servings } => {
            cmd_add(&cli.foods_dir, &cli.log_dir, &recipe, servings)?;
        }
        Commands::Today => {
            cmd_show(&cli.foods_dir, &cli.log_dir, Local::now().date_naive())?;
        }
        Commands::Log { date } => {
            let date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .context("date must be in YYYY-MM-DD format")?;
            cmd_show(&cli.foods_dir, &cli.log_dir, date)?;
        }
        Commands::Show { recipe } => {
            cmd_show_recipe(&cli.foods_dir, &recipe)?;
        }
        Commands::List => {
            cmd_list(&cli.foods_dir)?;
        }
        Commands::Fill { max_cal, min_protein, min_fiber, limit, exclude, include, max_servings, remaining, config } => {
            cmd_fill(&cli.foods_dir, &cli.log_dir, max_cal, min_protein, min_fiber, limit, &exclude, &include, max_servings, remaining, config)?;
        }
    }

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
    };

    let date = Local::now().date_naive();
    log::append_entry(log_dir, date, &entry)?;

    println!("Added {} servings of {} to {}", servings, recipe.title, date);
    println!();
    cmd_show(foods_dir, log_dir, date)?;

    Ok(())
}

struct ResolvedEntry {
    title: String,
    status: String,
}

fn resolve_entry(foods_dir: &Path, entry: &log::LogEntry) -> Result<ResolvedEntry> {
    let recipe_path = foods_dir.join(format!("{}.toml", entry.slug));
    let recipe = recipe::load_recipe(&recipe_path)
        .with_context(|| format!("recipe '{}' not found", entry.slug))?;
    let status = if recipe.hash() == entry.hash {
        String::new()
    } else {
        " ⚠ hash mismatch".to_string()
    };
    Ok(ResolvedEntry { title: recipe.title, status })
}

fn cmd_show(foods_dir: &Path, log_dir: &Path, date: chrono::NaiveDate) -> Result<()> {
    let day_log = log::load_day(log_dir, date)?;

    match day_log {
        None => println!("No entries for {}", date),
        Some(day_log) => {
            println!("{}", day_log.date);
            println!("{}", "-".repeat(60));
            let mut total_cal = 0.0;
            let mut total_protein = 0.0;
            let mut total_fiber = 0.0;

            for entry in &day_log.entries {
                let resolved = resolve_entry(foods_dir, entry)?;
                let cal = entry.calories as f64 * entry.servings;
                let prot = entry.protein_g * entry.servings;
                let fib = entry.fiber_g * entry.servings;

                let serv_str = if entry.servings.fract() == 0.0 {
                    format!("{:>4}", entry.servings as u32)
                } else {
                    format!("{:>4.1}", entry.servings)
                };
                println!(
                    "  {:<20} {} serv  {:>6.0} cal  {:>5.1}g prot  {:>4.1}g fiber{}",
                    resolved.title, serv_str, cal, prot, fib, resolved.status
                );

                total_cal += cal;
                total_protein += prot;
                total_fiber += fib;
            }

            println!("{}", "-".repeat(60));
            println!(
                "  {:<20}             {:>6.0} cal  {:>5.1}g prot  {:>4.1}g fiber",
                "TOTAL", total_cal, total_protein, total_fiber
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
