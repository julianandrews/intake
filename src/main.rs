use anyhow::{Context, Result};
use chrono::Local;
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::engine::ArgValueCandidates;
use clap_complete::{CompleteEnv, CompletionCandidate, Shell};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const CLAP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Green.on_default());

mod amount;
mod config;
mod display;
mod food;
mod log;
use amount::{Calories, Grams, Servings};
use config::{Column, Config};
use display::{ColumnValue, Table};
use rust_decimal::Decimal;

#[derive(Parser)]
#[command(name = "intake", color = clap::ColorChoice::Always, styles = CLAP_STYLES)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Directory containing food files
    #[arg(long)]
    foods_dir: Option<PathBuf>,

    /// Directory containing log files
    #[arg(long)]
    log_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a food to today's log
    Add {
        /// Food slug (filename without .toml)
        #[arg(add = ArgValueCandidates::new(complete_foods))]
        food: String,
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
        food: String,
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
        calories: u32,
    },
    /// Add an ad-hoc entry with custom macros (no food file needed)
    Adhoc {
        /// Name of the item
        name: String,
        /// Number of servings (default: 1)
        servings: Option<Servings>,
        /// Calories
        #[arg(long)]
        calories: Option<u32>,
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

fn complete_foods() -> Vec<CompletionCandidate> {
    let config = match completion_config() {
        Some(c) => c,
        None => return Vec::new(),
    };
    let dir = config.foods_dir();
    match food::list_food_slugs(&dir) {
        Ok(slugs) => slugs.into_iter().map(CompletionCandidate::new).collect(),
        Err(e) => {
            eprintln!("warning: failed to list foods for completion: {e}");
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
        Commands::Add { food, servings } => {
            cmd_add(&mut stdout, &foods_dir, &log_dir, &food, servings, &config)?;
        }
        Commands::Log { date, days_ago } => {
            let date = resolve_date(date, days_ago)?;
            cmd_log(&mut stdout, &log_dir, date, &config)?;
        }
        Commands::Show { food } => {
            cmd_show_food(&mut stdout, &foods_dir, &food, &config)?;
        }
        Commands::List => {
            cmd_list(&mut stdout, &foods_dir, &config)?;
        }
        Commands::Summary { date, days } => {
            let end = resolve_date(date, None)?;
            cmd_summary(&mut stdout, &log_dir, end, days, &config)?;
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
            fat,
            carbs,
            alcohol,
        } => {
            cmd_adhoc(
                &mut stdout,
                &log_dir,
                &name,
                servings.unwrap_or(Servings::from_u32(1)),
                &food::Macros {
                    calories: calories.map(Calories::from_u32).unwrap_or(Calories::ZERO),
                    protein_g: protein.unwrap_or(Grams::ZERO),
                    fiber_g: fiber.unwrap_or(Grams::ZERO),
                    fat_g: fat.unwrap_or(Grams::ZERO),
                    carbs_g: carbs.unwrap_or(Grams::ZERO),
                    alcohol_g: alcohol.unwrap_or(Grams::ZERO),
                },
            )?;
        }
    }

    Ok(())
}

fn cmd_adhoc(
    writer: &mut impl Write,
    log_dir: &Path,
    name: &str,
    servings: Servings,
    macros: &food::Macros,
) -> Result<()> {
    let entry = log::LogEntry {
        title: name.to_string(),
        servings,
        calories: macros.calories,
        protein_g: macros.protein_g,
        fiber_g: macros.fiber_g,
        fat_g: macros.fat_g,
        carbs_g: macros.carbs_g,
        alcohol_g: macros.alcohol_g,
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
    servings: Servings,
    config: &Config,
) -> Result<()> {
    let food_path = foods_dir.join(format!("{}.toml", slug));
    let food = food::load_food(&food_path).with_context(|| format!("food '{}' not found", slug))?;

    let ps = food.per_serving()?;

    let entry = log::LogEntry {
        title: food.title.clone(),
        servings,
        calories: ps.calories,
        protein_g: ps.protein_g,
        fiber_g: ps.fiber_g,
        fat_g: ps.fat_g,
        carbs_g: ps.carbs_g,
        alcohol_g: ps.alcohol_g,
    };

    let date = Local::now().date_naive();
    log::append_entry(log_dir, date, &entry)?;

    writeln!(
        writer,
        "Added {} servings of {} to {}",
        servings, food.title, date
    )?;
    writeln!(writer)?;
    cmd_log(writer, log_dir, date, config)?;

    Ok(())
}

fn resolve_date(date: Option<String>, days_ago: Option<u32>) -> Result<chrono::NaiveDate> {
    if let Some(d) = date {
        return chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
            .context("date must be in YYYY-MM-DD format");
    }
    let today = Local::now().date_naive();
    match days_ago {
        Some(n) => today
            .checked_sub_days(chrono::Days::new(n as u64))
            .context("days_ago exceeds the supported date span"),
        None => Ok(today),
    }
}

fn fmt_servings(servings: Decimal) -> String {
    if servings.fract().is_zero() {
        servings.round_dp(0).to_string()
    } else {
        servings.round_dp(1).to_string()
    }
}
fn cmd_log(
    writer: &mut impl Write,
    log_dir: &Path,
    date: chrono::NaiveDate,
    config: &Config,
) -> Result<()> {
    let day_log = log::load_day(log_dir, date)?;

    match day_log {
        None => writeln!(writer, "No entries for {}", date)?,
        Some(day_log) => {
            let columns = config.columns()?;
            let mut headers: Vec<&str> = vec!["Item", "Servings"];
            headers.extend(columns.iter().map(|c| c.label()));
            let mut table = Table::new(&headers);
            table.set_title(&date.to_string());

            let rows = build_rows(&day_log.entries)?;

            let mut totals = display::DayTotals::default();
            let mut total_servings = Decimal::ZERO;

            for row in &rows {
                let serv_str = fmt_servings(row.servings.to_decimal());

                let mut cells = vec![row.title.clone(), serv_str];
                for column in &columns {
                    cells.push(display::log_cell(*column, row.column_value(*column)));
                }
                table.add_row(cells);

                totals
                    .checked_add_row(row)
                    .context("day macro total overflow")?;
                total_servings = total_servings
                    .checked_add(row.servings.to_decimal())
                    .context("day servings total overflow")?;
            }

            let (net_cal, deficit) = day_net_and_deficit(
                totals.calories,
                day_log.exercise_calories,
                config.maintenance_calories,
            )?;

            let now = (date == Local::now().date_naive()).then(|| Local::now().time());
            let targets = config.targets()?;
            let show_exercise =
                day_log.exercise_calories > 0 && columns.contains(&Column::Calories);

            let mut plain_cells = Vec::new();
            let mut colored_cells = Vec::new();
            let mut exercise_cells = Vec::new();
            for column in &columns {
                let total = totals.column_value(*column);
                let colored = if *column == Column::Calories {
                    net_cal
                } else {
                    total
                };
                let color = display::column_color(now, colored, &targets.for_column(*column));
                plain_cells.push(display::log_cell(*column, total));
                colored_cells.push(display::wrap_color(
                    &display::log_cell(*column, colored),
                    color,
                ));
                if show_exercise {
                    exercise_cells.push(if *column == Column::Calories {
                        format!("-{}", day_log.exercise_calories)
                    } else {
                        String::new()
                    });
                }
            }

            let mut total_row = vec!["Total".to_string(), fmt_servings(total_servings)];
            if show_exercise {
                total_row.extend(plain_cells);
                table.add_footer_custom(total_row);

                let mut exercise_row = vec!["Exercise".to_string(), String::new()];
                exercise_row.extend(exercise_cells);
                table.add_footer_custom(exercise_row);

                let mut net_row = vec!["Net".to_string(), String::new()];
                net_row.extend(colored_cells);
                table.add_footer_custom(net_row);
            } else {
                total_row.extend(colored_cells);
                table.add_footer_custom(total_row);
            }

            write!(writer, "{}", table.format())?;

            let summary = display::render_day_summary(
                day_log.exercise_calories,
                config.maintenance_calories,
                deficit,
            );
            write!(writer, "{}", summary)?;
        }
    }

    Ok(())
}

fn day_net_and_deficit(
    calories: Decimal,
    exercise_calories: u32,
    maintenance_calories: Option<u32>,
) -> Result<(Decimal, Option<Decimal>)> {
    let exercise = Decimal::from(exercise_calories);
    let net_cal = calories
        .checked_sub(exercise)
        .context("net calorie total overflow")?;
    let deficit = maintenance_calories
        .map(|mc| {
            let tdee = Decimal::from(mc)
                .checked_add(exercise)
                .context("TDEE overflow")?;
            tdee.checked_sub(net_cal).context("deficit overflow")
        })
        .transpose()?;
    Ok((net_cal, deficit))
}

struct SummaryRow {
    date: chrono::NaiveDate,
    macros: display::DayTotals,
    exercise_calories: u32,
    deficit: Option<Decimal>,
}

impl ColumnValue for SummaryRow {
    fn column_value(&self, column: Column) -> Decimal {
        self.macros.column_value(column)
    }
}

fn build_summary_rows(
    log_dir: &Path,
    end: chrono::NaiveDate,
    days: u32,
    maintenance_calories: Option<u32>,
) -> Result<Vec<SummaryRow>> {
    if !log_dir.is_dir() {
        return Ok(Vec::new());
    }

    let days = days.max(1);
    let start = end
        .checked_sub_days(chrono::Days::new((days - 1) as u64))
        .context("days range exceeds the supported date span")?;

    let mut dates: Vec<chrono::NaiveDate> = log::list_log_dates(log_dir)?
        .into_iter()
        .filter_map(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .filter(|d| *d >= start && *d <= end)
        .collect();
    dates.sort_unstable();

    let mut rows = Vec::with_capacity(dates.len());
    for date in dates {
        if let Some(day_log) = log::load_day(log_dir, date)? {
            let mut macros = display::DayTotals::default();
            for entry in &day_log.entries {
                macros
                    .checked_add_row(&entry.totals()?)
                    .context("day macro total overflow")?;
            }

            let (_, deficit) = day_net_and_deficit(
                macros.calories,
                day_log.exercise_calories,
                maintenance_calories,
            )?;

            rows.push(SummaryRow {
                date,
                macros,
                exercise_calories: day_log.exercise_calories,
                deficit,
            });
        }
    }
    Ok(rows)
}

fn cmd_summary(
    writer: &mut impl Write,
    log_dir: &Path,
    end: chrono::NaiveDate,
    days: u32,
    config: &Config,
) -> Result<()> {
    let rows = build_summary_rows(log_dir, end, days, config.maintenance_calories)?;

    if rows.is_empty() {
        let window = days.max(1);
        writeln!(
            writer,
            "No entries in the last {} day{} (ending {})",
            window,
            if window == 1 { "" } else { "s" },
            end
        )?;
        return Ok(());
    }

    let columns = config.columns()?;
    let any_exercise =
        rows.iter().any(|r| r.exercise_calories > 0) && columns.contains(&Column::Calories);
    let show_deficit = config.maintenance_calories.is_some();

    let mut headers: Vec<&str> = vec!["Date"];
    headers.extend(columns.iter().map(|c| c.label()));
    if any_exercise {
        headers.push("Exercise");
    }
    if show_deficit {
        headers.push("Deficit");
    }

    let mut table = Table::new(&headers);
    table.set_title(&format!(
        "Summary {} to {}",
        rows.first().expect("rows checked non-empty").date,
        rows.last().expect("rows checked non-empty").date
    ));

    for row in &rows {
        let mut cells = vec![row.date.to_string()];
        for column in &columns {
            cells.push(display::log_cell(*column, row.column_value(*column)));
        }
        if any_exercise {
            if row.exercise_calories > 0 {
                cells.push(format!(
                    "{}{}{}",
                    display::ANSI_BOLD_RED,
                    row.exercise_calories,
                    display::ANSI_RESET
                ));
            } else {
                cells.push("0".to_string());
            }
        }
        if let Some(d) = row.deficit {
            cells.push(d.round_dp(0).to_string());
        }
        table.add_row(cells);
    }

    let count = Decimal::from(rows.len());
    let mut totals = display::DayTotals::default();
    for row in &rows {
        totals
            .checked_add_row(row)
            .context("period macro total overflow")?;
    }
    let total_exercise: u64 = rows.iter().map(|r| u64::from(r.exercise_calories)).sum();
    let total_deficit: Decimal = rows
        .iter()
        .try_fold(Decimal::ZERO, |acc, r| match r.deficit {
            Some(d) => acc.checked_add(d).context("period deficit overflow"),
            None => Ok(acc),
        })?;

    let mut total_footer = vec!["Total".to_string()];
    for column in &columns {
        total_footer.push(display::log_cell(*column, totals.column_value(*column)));
    }
    if any_exercise {
        total_footer.push(total_exercise.to_string());
    }
    if show_deficit {
        total_footer.push(total_deficit.round_dp(0).to_string());
    }
    table.add_footer(total_footer);

    let mut avg_footer = vec!["Avg/day".to_string()];
    for column in &columns {
        avg_footer.push(display::log_cell(
            *column,
            totals
                .column_value(*column)
                .checked_div(count)
                .expect("rows checked non-empty"),
        ));
    }
    if any_exercise {
        avg_footer.push(
            Decimal::from(total_exercise)
                .checked_div(count)
                .expect("rows checked non-empty")
                .round_dp(0)
                .to_string(),
        );
    }
    if show_deficit {
        avg_footer.push(
            total_deficit
                .checked_div(count)
                .expect("rows checked non-empty")
                .round_dp(0)
                .to_string(),
        );
    }
    table.add_footer(avg_footer);

    write!(writer, "{}", table.format())?;

    if !show_deficit {
        writeln!(writer)?;
        writeln!(writer, "Set maintenance_calories in config to see deficit.")?;
    }

    Ok(())
}

fn cmd_show_food(
    writer: &mut impl Write,
    foods_dir: &Path,
    slug: &str,
    config: &Config,
) -> Result<()> {
    let food_path = foods_dir.join(format!("{}.toml", slug));
    let food = food::load_food(&food_path).with_context(|| format!("food '{}' not found", slug))?;
    write!(writer, "{}", food.display(&config.columns()?)?)?;
    Ok(())
}

fn cmd_list(writer: &mut impl Write, foods_dir: &Path, config: &Config) -> Result<()> {
    let foods = food::find_all_foods(foods_dir)?;
    let columns = config.columns()?;

    let mut headers: Vec<&str> = vec!["Food", "Servings"];
    for column in &columns {
        headers.push(if *column == Column::Calories {
            "Cal/serv"
        } else {
            column.label()
        });
    }

    let mut table = Table::new(&headers);
    table.set_title("All Foods");

    for food in &foods {
        let ps = food.per_serving()?;
        let mut cells = vec![food.title.clone(), food.servings.to_string()];
        for column in &columns {
            cells.push(display::food_cell(*column, ps.column_value(*column)));
        }
        table.add_row(cells);
    }

    write!(writer, "{}", table.format())?;
    Ok(())
}

struct DisplayRow {
    title: String,
    servings: Servings,
    macros: display::DayTotals,
}

impl ColumnValue for DisplayRow {
    fn column_value(&self, column: Column) -> Decimal {
        self.macros.column_value(column)
    }
}

fn build_rows(entries: &[log::LogEntry]) -> Result<Vec<DisplayRow>> {
    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        rows.push(DisplayRow {
            title: entry.title.clone(),
            servings: entry.servings,
            macros: entry.totals()?,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LogEntry;

    fn entry(title: &str, servings: f64) -> LogEntry {
        LogEntry {
            title: title.to_string(),
            servings: Servings::from_f64(servings).unwrap(),
            calories: Calories::ZERO,
            protein_g: Grams::ZERO,
            fiber_g: Grams::ZERO,
            fat_g: Grams::ZERO,
            carbs_g: Grams::ZERO,
            alcohol_g: Grams::ZERO,
        }
    }

    #[test]
    fn test_build_rows_one_per_entry() {
        let entries = vec![
            entry("coffee", 1.0),
            entry("coffee", 2.0),
            entry("oatmeal", 1.0),
        ];
        let rows = build_rows(&entries).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn test_build_rows_titles_preserved() {
        let entries = vec![
            entry("Cherries - 155g", 1.0),
            entry("Sour Cream - 60g", 1.0),
        ];
        let rows = build_rows(&entries).unwrap();
        assert_eq!(rows[0].title, "Cherries - 155g");
        assert_eq!(rows[1].title, "Sour Cream - 60g");
    }

    #[test]
    fn test_build_rows_calories_scaled_by_servings() {
        let mut e = entry("coffee", 2.0);
        e.calories = Calories::from_u32(24);
        let rows = build_rows(&[e]).unwrap();
        assert_eq!(rows[0].macros.calories, Decimal::from(48));
    }

    #[test]
    fn test_build_rows_new_macros_scaled_by_servings() {
        let mut e = entry("coffee", 2.0);
        e.fat_g = Grams::from_f64(5.0).unwrap();
        e.carbs_g = Grams::from_f64(15.0).unwrap();
        e.alcohol_g = Grams::from_f64(2.0).unwrap();
        let rows = build_rows(&[e]).unwrap();
        assert_eq!(rows[0].macros.fat, Grams::from_f64(10.0).unwrap().into());
        assert_eq!(rows[0].macros.carbs, Grams::from_f64(30.0).unwrap().into());
        assert_eq!(rows[0].macros.alcohol, Grams::from_f64(4.0).unwrap().into());
    }

    #[test]
    fn test_resolve_date_defaults_to_today() {
        let today = Local::now().date_naive();
        let date = resolve_date(None, None).unwrap();
        assert_eq!(date, today);
    }

    #[test]
    fn test_resolve_date_days_ago_zero_is_today() {
        let today = Local::now().date_naive();
        let date = resolve_date(None, Some(0)).unwrap();
        assert_eq!(date, today);
    }

    #[test]
    fn test_resolve_date_days_ago_one_is_yesterday() {
        let today = Local::now().date_naive();
        let date = resolve_date(None, Some(1)).unwrap();
        assert_eq!(date, today - chrono::Days::new(1));
    }

    #[test]
    fn test_resolve_date_positional_wins() {
        let date = resolve_date(Some("2026-08-01".to_string()), Some(3)).unwrap();
        assert_eq!(date, chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
    }

    #[test]
    fn test_resolve_date_invalid_format_errors() {
        assert!(resolve_date(Some("yesterday".to_string()), None).is_err());
    }

    #[test]
    fn test_resolve_date_overflow_errors() {
        assert!(resolve_date(None, Some(u32::MAX)).is_err());
    }

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

    fn write_day_log(
        dir: &Path,
        date: chrono::NaiveDate,
        calories: f64,
        protein: f64,
        exercise: u32,
    ) {
        let day_log = log::DayLog {
            entries: vec![LogEntry {
                title: "coffee".to_string(),
                servings: Servings::from_f64(1.0).unwrap(),
                calories: Calories::from_f64(calories).unwrap(),
                protein_g: Grams::from_f64(protein).unwrap(),
                fiber_g: Grams::from_f64(4.0).unwrap(),
                fat_g: Grams::from_f64(2.0).unwrap(),
                carbs_g: Grams::from_f64(8.0).unwrap(),
                alcohol_g: Grams::ZERO,
            }],
            exercise_calories: exercise,
        };
        let content = toml::to_string(&day_log).unwrap();
        std::fs::write(dir.join(format!("{}.toml", date)), content).unwrap();
    }

    #[test]
    fn test_build_summary_rows_skips_empty_days() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, 100.0, 10.0, 0);
        write_day_log(dir.path(), end - chrono::Days::new(2), 200.0, 10.0, 0);

        let rows = build_summary_rows(dir.path(), end, 7, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, end - chrono::Days::new(2));
        assert_eq!(rows[1].date, end);
    }

    #[test]
    fn test_build_summary_rows_deficit_matches_day_math() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, 1500.0, 10.0, 300);

        let rows = build_summary_rows(dir.path(), end, 7, Some(2400)).unwrap();
        assert_eq!(rows.len(), 1);
        // net = 1500 - 300 = 1200; tdee = 2400 + 300 = 2700; deficit = 1500
        assert_eq!(rows[0].deficit, Some(Decimal::from(1500)));
    }

    #[test]
    fn test_build_summary_rows_empty_range() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let rows = build_summary_rows(dir.path(), end, 7, None).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_build_summary_rows_days_zero_clamped() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, 100.0, 10.0, 0);
        let rows = build_summary_rows(dir.path(), end, 0, None).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn test_build_summary_rows_days_overflow_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end, 100.0, 10.0, 0);
        let result = build_summary_rows(dir.path(), end, u32::MAX, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_summary_rows_ignores_non_date_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        write_day_log(dir.path(), end - chrono::Days::new(1), 100.0, 10.0, 0);
        std::fs::write(dir.path().join("README.toml"), "title = \"x\"\n").unwrap();
        let rows = build_summary_rows(dir.path(), end, 7, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, end - chrono::Days::new(1));
    }

    #[test]
    fn test_build_summary_rows_missing_log_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let rows = build_summary_rows(&dir.path().join("does-not-exist"), end, 7, None).unwrap();
        assert!(rows.is_empty());
    }
}
