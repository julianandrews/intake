use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use chrono::Local;
use std::path::{Path, PathBuf};

mod log;
mod recipe;

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
