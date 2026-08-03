use anyhow::{Context, Result};
use chrono::Local;
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::engine::ArgValueCandidates;
use clap_complete::{CompleteEnv, CompletionCandidate, Shell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const CLAP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Green.on_default());

mod config;
mod log;
mod recipe;
mod search;
use recipe::Table;

#[derive(Parser)]
#[command(name = "intake", color = clap::ColorChoice::Always, styles = CLAP_STYLES)]
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
    Today {
        /// Show entries as individual rows instead of grouping by recipe
        #[arg(long)]
        ungrouped: bool,
    },
    /// Show totals for a specific date (YYYY-MM-DD)
    Log {
        #[arg(add = ArgValueCandidates::new(complete_log_dates))]
        date: String,
        /// Show entries as individual rows instead of grouping by recipe
        #[arg(long)]
        ungrouped: bool,
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
    let dir = std::env::var("INTAKE_FOODS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("foods"));
    match recipe::list_recipe_slugs(&dir) {
        Ok(slugs) => slugs.into_iter().map(CompletionCandidate::new).collect(),
        Err(e) => {
            eprintln!("warning: failed to list recipes for completion: {e}");
            Vec::new()
        }
    }
}

fn complete_log_dates() -> Vec<CompletionCandidate> {
    let dir = std::env::var("INTAKE_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("log"));
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
            (
                base.join("bash-completion").join("completions"),
                "intake".to_string(),
            )
        }
        Shell::Zsh => {
            let base = std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                    PathBuf::from(home).join(".local").join("share")
                });
            (base.join("zsh").join("completions"), "_intake".to_string())
        }
        Shell::Fish => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            (
                PathBuf::from(home)
                    .join(".config")
                    .join("fish")
                    .join("completions"),
                "intake.fish".to_string(),
            )
        }
        _ => anyhow::bail!("install not supported for {} shell", shell),
    };
    Ok(dir.join(filename))
}

fn main() -> Result<()> {
    CompleteEnv::with_factory(Cli::command)
        .completer("intake")
        .complete();

    let cli = Cli::parse();

    let foods_dir = cli.foods_dir.unwrap_or_else(|| {
        std::env::var("INTAKE_FOODS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("foods"))
    });

    let log_dir = cli.log_dir.unwrap_or_else(|| {
        std::env::var("INTAKE_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("log"))
    });

    match cli.command {
        Commands::Add { recipe, servings } => {
            cmd_add(&foods_dir, &log_dir, &recipe, servings)?;
        }
        Commands::Today { ungrouped } => {
            cmd_show(&foods_dir, &log_dir, Local::now().date_naive(), ungrouped)?;
        }
        Commands::Log { date, ungrouped } => {
            let date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .context("date must be in YYYY-MM-DD format")?;
            cmd_show(&foods_dir, &log_dir, date, ungrouped)?;
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
                let completer = Path::new("intake");
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
                clap_complete::generate(shell, &mut cmd, "intake", &mut std::io::stdout());
            }
        }
        Commands::Adhoc {
            name,
            servings,
            calories,
            protein,
            fiber,
        } => {
            cmd_adhoc(
                &log_dir,
                &name,
                servings.unwrap_or(1.0),
                calories,
                protein,
                fiber,
            )?;
        }
        Commands::Fill {
            max_cal,
            min_protein,
            min_fiber,
            limit,
            exclude,
            include,
            max_servings,
            remaining,
            config,
        } => {
            cmd_fill(
                &foods_dir,
                &log_dir,
                max_cal,
                min_protein,
                min_fiber,
                limit,
                &exclude,
                &include,
                max_servings,
                remaining,
                config,
            )?;
        }
    }

    Ok(())
}

fn cmd_adhoc(
    log_dir: &Path,
    name: &str,
    servings: f64,
    calories: u32,
    protein_g: f64,
    fiber_g: f64,
) -> Result<()> {
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

    println!(
        "Added {} servings of {} to {}",
        servings, recipe.title, date
    );
    println!();
    cmd_show(foods_dir, log_dir, date, false)?;

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

fn cmd_show(
    foods_dir: &Path,
    log_dir: &Path,
    date: chrono::NaiveDate,
    ungrouped: bool,
) -> Result<()> {
    let day_log = log::load_day(log_dir, date)?;

    match day_log {
        None => println!("No entries for {}", date),
        Some(day_log) => {
            let mut table = Table::new(&["Item", "Servings", "Calories", "Protein(g)", "Fiber(g)"]);
            table.set_title(&date.to_string());

            let rows = if ungrouped {
                build_ungrouped_rows(foods_dir, &day_log.entries)?
            } else {
                build_grouped_rows(foods_dir, &day_log.entries)?
            };

            let mut total_cal = 0.0;
            let mut total_protein = 0.0;
            let mut total_fiber = 0.0;

            for row in &rows {
                let serv_str = if row.servings.fract() == 0.0 {
                    format!("{}", row.servings as u32)
                } else {
                    format!("{:.1}", row.servings)
                };

                table.add_row(vec![
                    row.title.clone(),
                    serv_str,
                    format!("{:.0}", row.calories),
                    format!("{:.1}", row.protein_g),
                    format!("{:.1}", row.fiber_g),
                ]);

                total_cal += row.calories;
                total_protein += row.protein_g;
                total_fiber += row.fiber_g;
            }

            table.add_footer(
                "TOTAL",
                vec![
                    String::new(),
                    format!("{:.0}", total_cal),
                    format!("{:.1}", total_protein),
                    format!("{:.1}", total_fiber),
                ],
            );

            println!("{}", table.format());
        }
    }

    Ok(())
}

struct DisplayRow {
    title: String,
    servings: f64,
    calories: f64,
    protein_g: f64,
    fiber_g: f64,
}

fn build_ungrouped_rows(foods_dir: &Path, entries: &[log::LogEntry]) -> Result<Vec<DisplayRow>> {
    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        let title = resolve_title(foods_dir, entry)?;
        rows.push(DisplayRow {
            title,
            servings: entry.servings,
            calories: entry.calories as f64 * entry.servings,
            protein_g: entry.protein_g * entry.servings,
            fiber_g: entry.fiber_g * entry.servings,
        });
    }
    Ok(rows)
}

fn build_grouped_rows(foods_dir: &Path, entries: &[log::LogEntry]) -> Result<Vec<DisplayRow>> {
    let mut rows: Vec<DisplayRow> = Vec::new();
    let mut slug_to_idx: HashMap<&str, usize> = HashMap::new();

    for entry in entries {
        if let Some(title) = entry.title.clone() {
            rows.push(DisplayRow {
                title,
                servings: entry.servings,
                calories: entry.calories as f64 * entry.servings,
                protein_g: entry.protein_g * entry.servings,
                fiber_g: entry.fiber_g * entry.servings,
            });
        } else if let Some(&idx) = slug_to_idx.get(entry.slug.as_str()) {
            let row = &mut rows[idx];
            row.servings += entry.servings;
            row.calories += entry.calories as f64 * entry.servings;
            row.protein_g += entry.protein_g * entry.servings;
            row.fiber_g += entry.fiber_g * entry.servings;
        } else {
            let title = resolve_title(foods_dir, entry)?;
            slug_to_idx.insert(entry.slug.as_str(), rows.len());
            rows.push(DisplayRow {
                title,
                servings: entry.servings,
                calories: entry.calories as f64 * entry.servings,
                protein_g: entry.protein_g * entry.servings,
                fiber_g: entry.fiber_g * entry.servings,
            });
        }
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LogEntry;

    fn foods_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("foods")
    }

    fn entry(slug: &str, servings: f64) -> LogEntry {
        LogEntry {
            slug: slug.to_string(),
            hash: String::new(),
            servings,
            calories: 0,
            protein_g: 0.0,
            fiber_g: 0.0,
            title: None,
        }
    }

    fn adhoc_entry(slug: &str, title: &str, servings: f64) -> LogEntry {
        LogEntry {
            slug: slug.to_string(),
            hash: String::new(),
            servings,
            calories: 0,
            protein_g: 0.0,
            fiber_g: 0.0,
            title: Some(title.to_string()),
        }
    }

    #[test]
    fn test_build_ungrouped_rows_one_per_entry() {
        let entries = vec![
            entry("coffee", 1.0),
            entry("coffee", 2.0),
            entry("oatmeal", 1.0),
        ];
        let rows = build_ungrouped_rows(&foods_dir(), &entries).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn test_build_grouped_rows_slugs_are_merged() {
        let entries = vec![
            entry("coffee", 1.0),
            entry("coffee", 2.0),
            entry("oatmeal", 1.0),
        ];
        let rows = build_grouped_rows(&foods_dir(), &entries).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter().find(|r| r.title == "Coffee").unwrap().servings,
            3.0
        );
        assert_eq!(
            rows.iter().find(|r| r.title == "Oatmeal").unwrap().servings,
            1.0
        );
    }

    #[test]
    fn test_build_grouped_rows_adhoc_entries_not_grouped() {
        let entries = vec![
            adhoc_entry("cherries---155g", "Cherries - 155g", 1.0),
            adhoc_entry("cherries---155g", "Cherries - 155g", 1.0),
        ];
        let rows = build_grouped_rows(&foods_dir(), &entries).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_build_grouped_rows_mixed_adhoc_and_recipe() {
        let entries = vec![
            entry("coffee", 1.0),
            entry("coffee", 1.0),
            adhoc_entry("sour-cream---60g", "Sour Cream - 60g", 1.0),
            entry("oatmeal", 2.0),
        ];
        let rows = build_grouped_rows(&foods_dir(), &entries).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter().find(|r| r.title == "Coffee").unwrap().servings,
            2.0
        );
        assert_eq!(
            rows.iter().find(|r| r.title == "Oatmeal").unwrap().servings,
            2.0
        );
        assert_eq!(
            rows.iter()
                .find(|r| r.title == "Sour Cream - 60g")
                .unwrap()
                .servings,
            1.0
        );
    }

    #[test]
    fn test_build_ungrouped_rows_calories_scaled_by_servings() {
        let mut e = entry("coffee", 2.0);
        e.calories = 24;
        let rows = build_ungrouped_rows(&foods_dir(), &[e]).unwrap();
        assert_eq!(rows[0].calories, 48.0);
    }

    #[test]
    fn test_build_grouped_rows_calories_accumulated() {
        let mut e1 = entry("coffee", 1.0);
        e1.calories = 24;
        let mut e2 = entry("coffee", 2.0);
        e2.calories = 24;
        let entries = vec![e1, e2];
        let rows = build_grouped_rows(&foods_dir(), &entries).unwrap();
        assert_eq!(rows[0].calories, 72.0);
    }
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

    let mut table = Table::new(&["Recipe", "Servings", "Cal/serv", "Protein(g)", "Fiber(g)"]);
    table.set_title("All Recipes");

    for (_, recipe) in &recipes {
        let ps = recipe.per_serving();
        table.add_row(vec![
            recipe.title.clone(),
            recipe.servings.to_string(),
            ps.calories.to_string(),
            format!("{:.1}g", ps.protein_g),
            format!("{:.1}g", ps.fiber_g),
        ]);
    }

    println!("{}", table.format());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
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
        let base = foods_dir
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        base.join("intake-config.toml")
    });

    let cfg_result = config::load_config(&config_path);
    let search_cfg = cfg_result.as_ref().ok();
    let max_nodes = search_cfg.map(|c| c.search.max_nodes).unwrap_or(100_000);
    let max_results = search_cfg.map(|c| c.search.max_results).unwrap_or(1000);

    let (max_calories, min_protein, min_fiber) = if remaining {
        let cfg = cfg_result.context("config required for --remaining mode (provide --config or place intake-config.toml alongside foods dir)")?;
        let goals = &cfg.goals;

        let today = chrono::Local::now().date_naive();
        let day_log = log::load_day(log_dir, today)?;
        let (consumed_cal, consumed_prot, consumed_fib) = match day_log {
            Some(log) => {
                let cal: u32 = log
                    .entries
                    .iter()
                    .map(|e| (e.calories as f64 * e.servings).round() as u32)
                    .sum();
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
