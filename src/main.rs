use anyhow::{Context, Result};
use chrono::Local;
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::engine::ArgValueCandidates;
use clap_complete::{CompleteEnv, CompletionCandidate, Shell};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const CLAP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Green.on_default());

mod config;
mod display;
mod log;
mod recipe;
use config::Config;
use display::Table;

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

fn completion_config() -> Option<&'static Config> {
    static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            Config::resolve(None, None).map(Some).unwrap_or_else(|e| {
                eprintln!("warning: failed to load config for completion: {e}");
                None
            })
        })
        .as_ref()
}

fn complete_recipes() -> Vec<CompletionCandidate> {
    let config = match completion_config() {
        Some(c) => c,
        None => return Vec::new(),
    };
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
    let config = match completion_config() {
        Some(c) => c,
        None => return Vec::new(),
    };
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
        Shell::Bash => (
            dirs::data_dir()
                .context("no data directory found")?
                .join("bash-completion")
                .join("completions"),
            "intake".to_string(),
        ),
        Shell::Zsh => (
            dirs::data_dir()
                .context("no data directory found")?
                .join("zsh")
                .join("completions"),
            "_intake".to_string(),
        ),
        Shell::Fish => (
            dirs::config_dir()
                .context("no config directory found")?
                .join("fish")
                .join("completions"),
            "intake.fish".to_string(),
        ),
        _ => anyhow::bail!("install not supported for {} shell", shell),
    };
    Ok(dir.join(filename))
}

fn main() -> Result<()> {
    CompleteEnv::with_factory(Cli::command)
        .completer("intake")
        .complete();

    let cli = Cli::parse();
    let config = Config::resolve(cli.foods_dir, cli.log_dir)?;
    let foods_dir = config.foods_dir();
    let log_dir = config.log_dir();
    let mut stdout = std::io::stdout();

    match cli.command {
        Commands::Add { recipe, servings } => {
            cmd_add(
                &mut stdout,
                &foods_dir,
                &log_dir,
                &recipe,
                servings,
                &config,
            )?;
        }
        Commands::Log { date, ungrouped } => {
            let date = match date {
                Some(d) => chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                    .context("date must be in YYYY-MM-DD format")?,
                None => Local::now().date_naive(),
            };
            cmd_log(&mut stdout, &foods_dir, &log_dir, date, ungrouped, &config)?;
        }
        Commands::Show { recipe } => {
            cmd_show_recipe(&mut stdout, &foods_dir, &recipe)?;
        }
        Commands::List => {
            cmd_list(&mut stdout, &foods_dir)?;
        }
        Commands::Exercise { calories } => {
            let date = Local::now().date_naive();
            log::set_exercise_calories(&log_dir, date, calories)?;
            writeln!(
                stdout,
                "Recorded {} exercise calories for {}",
                calories, date
            )?;
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
                writeln!(
                    stdout,
                    "Installed {} completions to {}",
                    shell,
                    path.display()
                )?;
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
                &mut stdout,
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
    writer: &mut impl Write,
    log_dir: &Path,
    name: &str,
    servings: f64,
    calories: u32,
    protein_g: f64,
    fiber_g: f64,
) -> Result<()> {
    let entry = log::LogEntry {
        slug: name.to_lowercase().replace(' ', "-"),
        servings,
        calories,
        protein_g,
        fiber_g,
        title: Some(name.to_string()),
    };

    let date = Local::now().date_naive();
    log::append_entry(log_dir, date, &entry)?;

    writeln!(
        writer,
        "Added {} serving(s) of {} to {}",
        servings, name, date
    )?;
    Ok(())
}

fn cmd_add(
    writer: &mut impl Write,
    foods_dir: &Path,
    log_dir: &Path,
    slug: &str,
    servings: f64,
    config: &Config,
) -> Result<()> {
    let recipe_path = foods_dir.join(format!("{}.toml", slug));
    let recipe = recipe::load_recipe(&recipe_path)
        .with_context(|| format!("recipe '{}' not found", slug))?;

    let ps = recipe.per_serving();

    let entry = log::LogEntry {
        slug: slug.to_string(),
        servings,
        calories: ps.calories,
        protein_g: ps.protein_g,
        fiber_g: ps.fiber_g,
        title: None,
    };

    let date = Local::now().date_naive();
    log::append_entry(log_dir, date, &entry)?;

    writeln!(
        writer,
        "Added {} servings of {} to {}",
        servings, recipe.title, date
    )?;
    writeln!(writer)?;
    cmd_log(writer, foods_dir, log_dir, date, false, config)?;

    Ok(())
}

fn resolve_title(
    foods_dir: &Path,
    entry: &log::LogEntry,
    cache: &mut HashMap<String, String>,
) -> Result<String> {
    if let Some(title) = &entry.title {
        return Ok(title.clone());
    }
    if let Some(title) = cache.get(&entry.slug) {
        return Ok(title.clone());
    }
    let recipe_path = foods_dir.join(format!("{}.toml", entry.slug));
    let recipe = recipe::load_recipe(&recipe_path)
        .with_context(|| format!("recipe '{}' not found", entry.slug))?;
    cache.insert(entry.slug.clone(), recipe.title.clone());
    Ok(recipe.title)
}

fn cmd_log(
    writer: &mut impl Write,
    foods_dir: &Path,
    log_dir: &Path,
    date: chrono::NaiveDate,
    ungrouped: bool,
    config: &Config,
) -> Result<()> {
    let day_log = log::load_day(log_dir, date)?;

    match day_log {
        None => writeln!(writer, "No entries for {}", date)?,
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

            write!(writer, "{}", table.format())?;

            let now = (date == Local::now().date_naive()).then(|| Local::now().time());

            let summary = display::render_day_summary(
                now,
                &display::DayTotals {
                    protein: total_protein,
                    fiber: total_fiber,
                },
                day_log.exercise_calories,
                net_cal,
                &display::DayTargets {
                    max_calories: config.max_calories,
                    min_protein: config.min_protein,
                    min_fiber: config.min_fiber,
                    maintenance_calories: config.maintenance_calories,
                },
                deficit,
            );
            write!(writer, "{}", summary)?;
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
    let mut cache = HashMap::new();
    for entry in entries {
        let title = resolve_title(foods_dir, entry, &mut cache)?;
        rows.push(DisplayRow {
            title,
            servings: entry.servings,
            calories: entry.total_calories(),
            protein_g: entry.total_protein(),
            fiber_g: entry.total_fiber(),
        });
    }
    Ok(rows)
}

fn build_grouped_rows(foods_dir: &Path, entries: &[log::LogEntry]) -> Result<Vec<DisplayRow>> {
    let mut rows: Vec<DisplayRow> = Vec::new();
    let mut slug_to_idx: HashMap<&str, usize> = HashMap::new();
    let mut cache = HashMap::new();

    for entry in entries {
        if let Some(title) = entry.title.clone() {
            rows.push(DisplayRow {
                title,
                servings: entry.servings,
                calories: entry.total_calories(),
                protein_g: entry.total_protein(),
                fiber_g: entry.total_fiber(),
            });
        } else if let Some(&idx) = slug_to_idx.get(entry.slug.as_str()) {
            let row = &mut rows[idx];
            row.servings += entry.servings;
            row.calories += entry.total_calories();
            row.protein_g += entry.total_protein();
            row.fiber_g += entry.total_fiber();
        } else {
            let title = resolve_title(foods_dir, entry, &mut cache)?;
            slug_to_idx.insert(entry.slug.as_str(), rows.len());
            rows.push(DisplayRow {
                title,
                servings: entry.servings,
                calories: entry.total_calories(),
                protein_g: entry.total_protein(),
                fiber_g: entry.total_fiber(),
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

fn cmd_show_recipe(writer: &mut impl Write, foods_dir: &Path, slug: &str) -> Result<()> {
    let recipe_path = foods_dir.join(format!("{}.toml", slug));
    let recipe = recipe::load_recipe(&recipe_path)
        .with_context(|| format!("recipe '{}' not found", slug))?;
    write!(writer, "{}", recipe.display())?;
    Ok(())
}

fn cmd_list(writer: &mut impl Write, foods_dir: &Path) -> Result<()> {
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

    write!(writer, "{}", table.format())?;
    Ok(())
}
