use anyhow::{Context, Result};
use chrono::{Local, Timelike};
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::engine::ArgValueCandidates;
use clap_complete::{CompleteEnv, CompletionCandidate, Shell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_BOLD_RED: &str = "\x1b[1;31m";

const CLAP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Green.on_default());

mod config;
mod log;
mod recipe;
use config::Config;
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
    /// Show totals for a date (default: today)
    Log {
        #[arg(add = ArgValueCandidates::new(complete_log_dates))]
        date: Option<String>,
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
    /// Record exercise calories for today
    Exercise {
        /// Calories burned
        calories: u32,
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
}

fn complete_recipes() -> Vec<CompletionCandidate> {
    let config = Config::resolve();
    let dir = config.foods_dir();
    match recipe::list_recipe_slugs(&dir) {
        Ok(slugs) => slugs.into_iter().map(CompletionCandidate::new).collect(),
        Err(e) => {
            eprintln!("warning: failed to list recipes for completion: {e}");
            Vec::new()
        }
    }
}

fn complete_log_dates() -> Vec<CompletionCandidate> {
    let config = Config::resolve();
    let dir = config.log_dir();
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
    let config = Config::resolve().with_cli_overrides(cli.foods_dir, cli.log_dir);
    let foods_dir = config.foods_dir();
    let log_dir = config.log_dir();

    match cli.command {
        Commands::Add { recipe, servings } => {
            cmd_add(&foods_dir, &log_dir, &recipe, servings, &config)?;
        }
        Commands::Log { date, ungrouped } => {
            let date = match date {
                Some(d) => chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                    .context("date must be in YYYY-MM-DD format")?,
                None => Local::now().date_naive(),
            };
            cmd_show(&foods_dir, &log_dir, date, ungrouped, &config)?;
        }
        Commands::Show { recipe } => {
            cmd_show_recipe(&foods_dir, &recipe)?;
        }
        Commands::List => {
            cmd_list(&foods_dir)?;
        }
        Commands::Exercise { calories } => {
            let date = Local::now().date_naive();
            log::set_exercise_calories(&log_dir, date, calories)?;
            println!("Recorded {} exercise calories for {}", calories, date);
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

fn cmd_add(
    foods_dir: &Path,
    log_dir: &Path,
    slug: &str,
    servings: f64,
    config: &Config,
) -> Result<()> {
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
    cmd_show(foods_dir, log_dir, date, false, config)?;

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

fn day_proportion() -> f64 {
    let now = Local::now();
    let elapsed = now.hour() * 3600 + now.minute() * 60 + now.second();
    elapsed as f64 / 86400.0
}

fn cmd_show(
    foods_dir: &Path,
    log_dir: &Path,
    date: chrono::NaiveDate,
    ungrouped: bool,
    config: &Config,
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
                "Total",
                vec![
                    String::new(),
                    format!("{:.0}", total_cal),
                    format!("{:.1}", total_protein),
                    format!("{:.1}", total_fiber),
                ],
            );

            let net_cal = total_cal - day_log.exercise_calories as f64;

            let deficit = config.maintenance_calories.map(|mc| {
                let tdee = mc as f64 + day_log.exercise_calories as f64;
                tdee - net_cal
            });

            println!("{}", table.format());

            let is_today = date == Local::now().date_naive();
            let dp = day_proportion();

            let mut line1: Vec<String> = Vec::new();

            if let Some(target) = config.max_calories {
                let color = if net_cal > target as f64 {
                    ANSI_BOLD_RED
                } else if !is_today {
                    ANSI_BOLD_GREEN
                } else {
                    ANSI_BOLD_YELLOW
                };
                line1.push(format!(
                    "Calories: {color}{:.0}{}/{}",
                    net_cal, ANSI_RESET, target
                ));
            }
            if let Some(target) = config.min_protein {
                let color = if total_protein >= target {
                    ANSI_BOLD_GREEN
                } else if !is_today {
                    ANSI_BOLD_RED
                } else {
                    let ratio = total_protein / target;
                    if ratio >= dp {
                        ANSI_BOLD_YELLOW
                    } else {
                        ANSI_BOLD_RED
                    }
                };
                line1.push(format!(
                    "Protein: {color}{:.1}{}/{}g",
                    total_protein, ANSI_RESET, target
                ));
            }
            if let Some(target) = config.min_fiber {
                let color = if total_fiber >= target {
                    ANSI_BOLD_GREEN
                } else if !is_today {
                    ANSI_BOLD_RED
                } else {
                    let ratio = total_fiber / target;
                    if ratio >= dp {
                        ANSI_BOLD_YELLOW
                    } else {
                        ANSI_BOLD_RED
                    }
                };
                line1.push(format!(
                    "Fiber: {color}{:.1}{}/{}g",
                    total_fiber, ANSI_RESET, target
                ));
            }

            let mut line2: Vec<String> = Vec::new();

            if day_log.exercise_calories > 0 {
                line2.push(format!(
                    "Exercise: {ANSI_BOLD_RED}{}{ANSI_RESET}",
                    day_log.exercise_calories
                ));
            }
            if let Some(mc) = config.maintenance_calories {
                let tdee = mc + day_log.exercise_calories;
                line2.push(format!("TDEE: {}", tdee));
            }
            if let Some(d) = deficit {
                let color = if d >= 0.0 {
                    ANSI_BOLD_GREEN
                } else {
                    ANSI_BOLD_RED
                };
                line2.push(format!("Deficit: {color}{:.0}{}", d, ANSI_RESET));
            }

            let max_len = line1.len().max(line2.len());
            for i in 0..max_len {
                let w1 = line1.get(i).map(|s| recipe::visible_width(s)).unwrap_or(0);
                let w2 = line2.get(i).map(|s| recipe::visible_width(s)).unwrap_or(0);
                let mw = w1.max(w2);
                if let Some(s) = line1.get_mut(i) {
                    let pad = mw - recipe::visible_width(s);
                    for _ in 0..pad {
                        s.push(' ');
                    }
                }
                if let Some(s) = line2.get_mut(i) {
                    let pad = mw - recipe::visible_width(s);
                    for _ in 0..pad {
                        s.push(' ');
                    }
                }
            }

            if !line1.is_empty() {
                println!("  {}", line1.join("    "));
            }
            if !line2.is_empty() {
                println!("  {}", line2.join("    "));
            }
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foods")
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
