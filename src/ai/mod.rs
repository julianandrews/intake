mod catalog;
pub(crate) mod cli;
mod confirm;
mod context;
mod food_lookup;
mod ops;
mod prompts;
pub(crate) mod settings;
mod spinner;
mod write;

use crate::amount::Calories;
use crate::commands;
use crate::config::Config;
use crate::confirm::nothing_confirmed;
use crate::editor;
use crate::food::{self, FoodName};
use crate::log;
use anyhow::{bail, Context, Result};
use intake_ai::confirm::Confirmer;
use intake_ai::llm::{LlmBackend, OpenAiCompatible};
use intake_ai::pipeline::{ResolveContext, ResolveError, Resolver};
use intake_ai::settings::AiSettings;
use intake_ai::tools::Tool;
use intake_ai::usda::{UsdaGet, UsdaSearch};
use similar::{ChangeTag, TextDiff};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

pub(crate) fn run(
    writer: &mut impl Write,
    foods_dir: &Path,
    log_dir: &Path,
    command: cli::AiCommands,
    config: &Config,
) -> Result<()> {
    match command {
        cli::AiCommands::Log {
            prompt,
            date,
            flags,
        } => {
            let date = commands::resolve_log_date(date.date)?;
            let Some(prompt) = capture_prompt(&prompt, flags.prompt_arg.as_deref())? else {
                return write_nothing(writer);
            };
            let session = AiSession::new(config, &flags)?;
            cmd_ai_log(
                writer,
                &session.env(config),
                foods_dir,
                log_dir,
                date,
                &prompt,
                |w| confirmer_for(w, flags.yes),
            )
        }
        cli::AiCommands::Food {
            command:
                cli::AiFoodCommands::New {
                    name,
                    prompt,
                    flags,
                },
        } => {
            let Some(prompt) = capture_prompt(&prompt, flags.prompt_arg.as_deref())? else {
                return write_nothing(writer);
            };
            let session = AiSession::new(config, &flags)?;
            cmd_ai_food_new(
                writer,
                &session.env(config),
                foods_dir,
                &name,
                &prompt,
                |w| confirmer_for(w, flags.yes),
            )
        }
        cli::AiCommands::Food {
            command:
                cli::AiFoodCommands::Edit {
                    name,
                    prompt,
                    flags,
                },
        } => {
            let Some(prompt) = capture_prompt(&prompt, flags.prompt_arg.as_deref())? else {
                return write_nothing(writer);
            };
            let session = AiSession::new(config, &flags)?;
            cmd_ai_food_edit(
                writer,
                &session.env(config),
                foods_dir,
                &name,
                &prompt,
                |w| confirmer_for(w, flags.yes),
            )
        }
    }
}

fn confirmer_for<'a>(writer: &'a mut dyn Write, yes: bool) -> Box<dyn Confirmer + 'a> {
    if yes {
        Box::new(confirm::ConfirmAlways)
    } else {
        Box::new(confirm::AiConfirmer::new(writer))
    }
}

fn write_nothing(writer: &mut impl Write) -> Result<()> {
    writeln!(writer, "Nothing written")?;
    Ok(())
}

fn handle_error(writer: &mut impl Write, err: ResolveError) -> Result<()> {
    match err {
        ResolveError::Rejected => write_nothing(writer),
        ResolveError::Cancelled => nothing_confirmed(writer, "written"),
        ResolveError::Exhausted {
            last_error,
            raw_output,
        } => bail!(
            "AI could not produce valid output: {last_error}\n\nLast model output:\n{raw_output}"
        ),
        ResolveError::Io(e) => Err(e),
    }
}

fn capture_prompt(prompt_words: &[String], inline: Option<&str>) -> Result<Option<String>> {
    if let Some(inline) = inline {
        return Ok(Some(inline.to_string()));
    }
    if !prompt_words.is_empty() {
        return Ok(Some(prompt_words.join(" ")));
    }
    let prefill = "# Describe the change you want.\n# Leave the file unchanged to abort.\n";
    editor::capture_via_editor(prefill, ".md")
}

fn resolve_settings(config: &Config, flags: &cli::AiFlags) -> Result<AiSettings> {
    let from_config = config.ai.as_ref().map(|ai| ai.settings.clone());
    let mut settings = from_config.unwrap_or_default();
    for (key, target) in [
        ("INTAKE_AI_API_KEY", &mut settings.api_key),
        ("INTAKE_AI_MODEL", &mut settings.model),
    ] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                *target = value;
            }
        }
    }
    if let Ok(value) = std::env::var("INTAKE_AI_USDA_API_KEY") {
        if !value.is_empty() {
            settings.usda_api_key = Some(value);
        }
    }
    if let Some(value) = &flags.api_key {
        settings.api_key = value.clone();
    }
    if let Some(value) = &flags.model {
        settings.model = value.clone();
    }
    settings.trace_requests = settings.trace_requests || flags.trace_requests;
    settings.trace_responses = settings.trace_responses || flags.trace_responses;
    settings.trace_colors = crate::display::color_enabled();
    Ok(settings)
}

fn prompt_override(
    config: &Config,
    key: fn(&settings::AiConfig) -> &Option<String>,
) -> Option<&str> {
    config.ai.as_ref().and_then(|ai| key(ai).as_deref())
}

fn system_prompt(default: &str, override_prompt: Option<&str>, context: &str) -> String {
    let mut text = override_prompt.unwrap_or(default).to_string();
    if !context.is_empty() {
        text.push_str("\n\n");
        text.push_str(context);
    }
    text
}

struct AiEnv<'a> {
    settings: &'a AiSettings,
    backend: &'a dyn LlmBackend,
    config: &'a Config,
}

/// Resolved settings plus the backend they build, owning both so an [`AiEnv`]
/// can borrow them for the duration of a command. Constructed per command
/// from the config file → env var → CLI flag resolution in
/// [`resolve_settings`].
struct AiSession {
    settings: AiSettings,
    backend: OpenAiCompatible,
}

impl AiSession {
    fn new(config: &Config, flags: &cli::AiFlags) -> Result<AiSession> {
        let settings = resolve_settings(config, flags)?;
        let backend = OpenAiCompatible::new(&settings);
        Ok(AiSession { settings, backend })
    }

    fn env<'a>(&'a self, config: &'a Config) -> AiEnv<'a> {
        AiEnv {
            settings: &self.settings,
            backend: &self.backend,
            config,
        }
    }
}

fn usda_tools(settings: &AiSettings) -> (UsdaSearch, UsdaGet) {
    let key = settings.usda_api_key.as_deref().unwrap_or("");
    let timeout = Duration::from_secs(settings.usda_timeout_secs);
    (UsdaSearch::new(key, timeout), UsdaGet::new(key, timeout))
}

fn sample_foods_context(foods_dir: &Path) -> Result<String> {
    let samples = context::sample_foods(foods_dir)?;
    if samples.is_empty() {
        let canned = toml::to_string(&commands::food::canned_example_food())
            .context("failed to serialize example food")?;
        return Ok(format!(
            "Example food (your catalog is empty — follow these conventions):\n{canned}"
        ));
    }
    let mut out = String::from("Sample foods from your catalog (follow their conventions):\n");
    for sample in samples {
        out.push_str(&toml::to_string(&sample).context("failed to serialize sample food")?);
        out.push('\n');
    }
    Ok(out)
}

fn row_diff(before: &[log::LogEntry], after: &[log::LogEntry]) -> String {
    let before_text = before
        .iter()
        .map(context::entry_line)
        .collect::<Vec<_>>()
        .join("\n");
    let after_text = after
        .iter()
        .map(context::entry_line)
        .collect::<Vec<_>>()
        .join("\n");
    let diff = TextDiff::from_lines(&before_text, &after_text);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => continue,
        };
        out.push_str(&format!("{sign} {}\n", change.value().trim_end()));
    }
    out
}

/// The parse-and-present payload of a resolve session: the system prompt
/// and user prompt, plus the closures that turn the model's text into a
/// value and render it for confirmation.
struct ResolveJob<'a, T> {
    system: String,
    prompt: &'a str,
    parse: &'a dyn Fn(&str) -> Result<T, String>,
    present: &'a dyn Fn(&T) -> String,
}

/// Runs a resolve session under a status line: wraps the backend, tools,
/// and confirmer with the spinner decorators, then drives the resolver.
/// The caller creates the line — it may need it to wire tool warnings —
/// and it is erased when it drops after the session.
fn resolve_session<T>(
    env: &AiEnv<'_>,
    confirmer: Box<dyn Confirmer + '_>,
    tools: &[&dyn Tool],
    status: &spinner::StatusLine,
    job: ResolveJob<'_, T>,
) -> Result<T, ResolveError> {
    let backend = spinner::SpinnerBackend::new(env.backend, env.settings, status);
    let wrapped: Vec<spinner::StatusTool<'_>> = tools
        .iter()
        .map(|t| spinner::StatusTool::new(*t, status))
        .collect();
    let wrapped: Vec<&dyn Tool> = wrapped.iter().map(|t| t as &dyn Tool).collect();
    let mut confirmer = spinner::SpinnerConfirmer::new(confirmer, status);
    let ctx = ResolveContext {
        settings: env.settings,
        backend: &backend,
        tools: &wrapped,
        trace_sink: Some(status.sink()),
    };
    let mut resolver = Resolver::new(&ctx, &mut confirmer, job.system);
    resolver.resolve(job.prompt, job.parse, job.present)
}

fn cmd_ai_log(
    writer: &mut impl Write,
    env: &AiEnv<'_>,
    foods_dir: &Path,
    log_dir: &Path,
    date: chrono::NaiveDate,
    prompt: &str,
    make_confirmer: impl FnOnce(&mut dyn Write) -> Box<dyn Confirmer + '_>,
) -> Result<()> {
    let settings = env.settings;
    let config = env.config;
    let original = log::load_day(log_dir, date)?;
    let context_text = context::day_context(date, original.as_ref(), log_dir, config)?;
    let system = system_prompt(
        prompts::LOG,
        prompt_override(config, |ai| &ai.log_prompt),
        &context_text,
    );
    let (usda_search, usda_get) = usda_tools(settings);
    let status = spinner::StatusLine::new(settings);
    let warn = |msg: &str| status.warn(msg);
    let food_lookup = food_lookup::FoodLookup::new(foods_dir).with_warn(&warn);

    let base = original.clone().unwrap_or(log::DayLog {
        entries: Vec::new(),
        exercise_calories: Calories::ZERO,
    });
    let parse = |s: &str| -> Result<log::DayLog, String> {
        let day_ops: ops::DayLogOps = toml::from_str(s).map_err(|e| e.to_string())?;
        ops::apply_ops(&base, &day_ops.ops, foods_dir)
    };
    let present = |applied: &log::DayLog| -> String {
        let diff = row_diff(
            original
                .as_ref()
                .map(|d| d.entries.as_slice())
                .unwrap_or(&[]),
            &applied.entries,
        );
        if diff.is_empty() {
            return confirm::NO_CHANGES_PROPOSAL.to_string();
        }
        let table = crate::commands::log::render_day(applied, date, config)
            .unwrap_or_else(|e| format!("(day table unavailable: {e})"));
        format!("{diff}\n\n{table}")
    };

    let outcome = {
        let confirmer = make_confirmer(writer);
        let tools: [&dyn Tool; 3] = [&food_lookup, &usda_search, &usda_get];
        resolve_session(
            env,
            confirmer,
            &tools,
            &status,
            ResolveJob {
                system,
                prompt,
                parse: &parse,
                present: &present,
            },
        )
    };

    match outcome {
        Ok(applied) => {
            let changed = match &original {
                Some(d) => d != &applied,
                None => {
                    !(applied.entries.is_empty() && applied.exercise_calories == Calories::ZERO)
                }
            };
            if !changed {
                writeln!(writer, "No changes")?;
                return Ok(());
            }
            write::write_day_checked(log_dir, date, original.as_ref(), applied)?;
            writeln!(writer)?;
            crate::commands::log::cmd_day(writer, log_dir, date, config)?;
        }
        Err(err) => {
            status.pause();
            handle_error(writer, err)?
        }
    }
    Ok(())
}

fn cmd_ai_food_new(
    writer: &mut impl Write,
    env: &AiEnv<'_>,
    foods_dir: &Path,
    name: &FoodName,
    prompt: &str,
    make_confirmer: impl FnOnce(&mut dyn Write) -> Box<dyn Confirmer + '_>,
) -> Result<()> {
    let settings = env.settings;
    let config = env.config;
    let path = name.file_path(foods_dir);
    if path.exists() {
        bail!(
            "food '{}' already exists — use `food edit {}` / `ai food edit {}` to modify it",
            name,
            name,
            name
        );
    }
    let samples = sample_foods_context(foods_dir)?;
    let system = system_prompt(
        prompts::FOOD_NEW,
        prompt_override(config, |ai| &ai.food_new_prompt),
        &samples,
    );
    let (usda_search, usda_get) = usda_tools(settings);
    let status = spinner::StatusLine::new(settings);
    let columns = config.columns()?;

    let parse =
        |s: &str| -> Result<food::Food, String> { toml::from_str(s).map_err(|e| e.to_string()) };
    let present = |f: &food::Food| -> String {
        crate::commands::food::render_food(f, &columns)
            .unwrap_or_else(|e| format!("(unavailable: {e})"))
    };

    let outcome = {
        let confirmer = make_confirmer(writer);
        let tools: [&dyn Tool; 2] = [&usda_search, &usda_get];
        resolve_session(
            env,
            confirmer,
            &tools,
            &status,
            ResolveJob {
                system,
                prompt,
                parse: &parse,
                present: &present,
            },
        )
    };

    match outcome {
        Ok(new_food) => {
            food::create_food(foods_dir, name, &new_food)?;
            writeln!(writer, "Wrote {}", path.display())?;
            writeln!(writer)?;
            write!(
                writer,
                "{}",
                crate::commands::food::render_food(&new_food, &columns)?
            )?;
        }
        Err(err) => {
            status.pause();
            handle_error(writer, err)?
        }
    }
    Ok(())
}

fn cmd_ai_food_edit(
    writer: &mut impl Write,
    env: &AiEnv<'_>,
    foods_dir: &Path,
    name: &FoodName,
    prompt: &str,
    make_confirmer: impl FnOnce(&mut dyn Write) -> Box<dyn Confirmer + '_>,
) -> Result<()> {
    let settings = env.settings;
    let config = env.config;
    let path = name.file_path(foods_dir);
    let original = food::load_food(&path).with_context(|| format!("food '{}' not found", name))?;
    let mut context_text = String::from("Current food:\n");
    context_text.push_str(&toml::to_string(&original).context("failed to serialize food")?);
    context_text.push('\n');
    context_text.push_str(&sample_foods_context(foods_dir)?);
    let system = system_prompt(
        prompts::FOOD_EDIT,
        prompt_override(config, |ai| &ai.food_edit_prompt),
        &context_text,
    );
    let (usda_search, usda_get) = usda_tools(settings);
    let status = spinner::StatusLine::new(settings);
    let columns = config.columns()?;

    let parse =
        |s: &str| -> Result<food::Food, String> { toml::from_str(s).map_err(|e| e.to_string()) };
    let present = |f: &food::Food| -> String {
        let before = crate::commands::food::render_food(&original, &columns)
            .unwrap_or_else(|e| format!("(unavailable: {e})"));
        let after = crate::commands::food::render_food(f, &columns)
            .unwrap_or_else(|e| format!("(unavailable: {e})"));
        format!("Before:\n{before}\nAfter:\n{after}")
    };

    let outcome = {
        let confirmer = make_confirmer(writer);
        let tools: [&dyn Tool; 2] = [&usda_search, &usda_get];
        resolve_session(
            env,
            confirmer,
            &tools,
            &status,
            ResolveJob {
                system,
                prompt,
                parse: &parse,
                present: &present,
            },
        )
    };

    match outcome {
        Ok(new_food) => {
            let current =
                food::load_food(&path).with_context(|| format!("food '{}' not found", name))?;
            if current != original {
                bail!(
                    "food '{}' changed since this proposal was generated — re-run",
                    name
                );
            }
            if new_food == original {
                writeln!(writer, "No changes")?;
                return Ok(());
            }
            food::write_food(foods_dir, name, &new_food)?;
            writeln!(writer, "Wrote {}", path.display())?;
            writeln!(writer)?;
            write!(
                writer,
                "{}",
                crate::commands::food::render_food(&new_food, &columns)?
            )?;
        }
        Err(err) => {
            status.pause();
            handle_error(writer, err)?
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::{Grams, Servings};
    use intake_ai::confirm::{ConfirmDecision, ConfirmError};
    use intake_ai::llm::{AssistantMessage, LlmError, Message};
    use std::str::FromStr;
    use std::sync::Mutex;

    struct FakeBackend {
        queue: Mutex<Vec<Result<AssistantMessage, LlmError>>>,
    }

    impl FakeBackend {
        fn new(responses: Vec<Result<AssistantMessage, LlmError>>) -> FakeBackend {
            FakeBackend {
                queue: Mutex::new(responses),
            }
        }
    }

    impl LlmBackend for FakeBackend {
        fn complete(
            &self,
            _messages: &[Message],
            _tools: &[&dyn Tool],
        ) -> Result<AssistantMessage, LlmError> {
            let mut queue = self.queue.lock().unwrap();
            if queue.is_empty() {
                return Err(LlmError::Transport("no more responses".to_string()));
            }
            queue.remove(0)
        }
    }

    struct Scripted<'a> {
        queue: Mutex<Vec<Result<ConfirmDecision, ConfirmError>>>,
        present: bool,
        out: Option<&'a mut dyn Write>,
    }

    impl<'a> Scripted<'a> {
        fn new(decisions: Vec<ConfirmDecision>) -> Scripted<'a> {
            Scripted {
                queue: Mutex::new(decisions.into_iter().map(Ok).collect()),
                present: true,
                out: None,
            }
        }

        fn results(results: Vec<Result<ConfirmDecision, ConfirmError>>) -> Scripted<'a> {
            Scripted {
                queue: Mutex::new(results),
                present: true,
                out: None,
            }
        }

        fn with_writer(writer: &'a mut dyn Write, decisions: Vec<ConfirmDecision>) -> Scripted<'a> {
            Scripted {
                queue: Mutex::new(decisions.into_iter().map(Ok).collect()),
                present: true,
                out: Some(writer),
            }
        }

        fn skip_present(decisions: Vec<ConfirmDecision>) -> Scripted<'a> {
            Scripted {
                queue: Mutex::new(decisions.into_iter().map(Ok).collect()),
                present: false,
                out: None,
            }
        }
    }

    impl Confirmer for Scripted<'_> {
        fn confirm(&mut self, rendered: &str) -> Result<ConfirmDecision, ConfirmError> {
            if let Some(writer) = self.out.as_deref_mut() {
                writer.write_all(rendered.as_bytes()).ok();
            }
            let mut queue = self.queue.lock().unwrap();
            if queue.is_empty() {
                Ok(ConfirmDecision::Reject)
            } else {
                queue.remove(0)
            }
        }

        fn present_before_confirm(&self) -> bool {
            self.present
        }
    }

    fn settings() -> AiSettings {
        AiSettings {
            max_retries: 3,
            max_tool_calls: 20,
            ..AiSettings::default()
        }
    }

    fn entry(title: &str, servings: &str, calories: &str) -> log::LogEntry {
        log::LogEntry {
            title: title.to_string(),
            servings: Servings::from_str(servings).unwrap(),
            calories: crate::amount::Calories::from_str(calories).unwrap(),
            protein_g: Grams::from_str("0").unwrap(),
            fiber_g: Grams::from_str("0").unwrap(),
            fat_g: Grams::from_str("0").unwrap(),
            carbs_g: Grams::from_str("0").unwrap(),
            alcohol_g: Grams::from_str("0").unwrap(),
        }
    }

    fn write_day(dir: &Path, date: chrono::NaiveDate, entries: Vec<log::LogEntry>) {
        let day = log::DayLog {
            entries,
            exercise_calories: crate::amount::Calories::ZERO,
        };
        std::fs::write(
            dir.join(format!("{}.toml", date.format("%Y-%m-%d"))),
            toml::to_string(&day).unwrap(),
        )
        .unwrap();
    }

    fn foods_dir() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("oatmeal.toml"),
            "title = \"Oatmeal\"\nservings = 1\n\n[[ingredients]]\nname = \"Oats\"\nquantity = \"100g\"\ncalories = 200\nprotein_g = 10\nfiber_g = 5\nfat_g = 4\ncarbs_g = 30\nalcohol_g = 0\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn test_row_diff_one_macro_changed_one_pair() {
        let mut after = entry("coffee", "1", "12");
        after.calories = crate::amount::Calories::from_str("13").unwrap();
        let diff = row_diff(&[entry("coffee", "1", "12")], &[after]);
        assert_eq!(
            diff,
            "- coffee | 1 | 12, 0, 0, 0, 0, 0\n+ coffee | 1 | 13, 0, 0, 0, 0, 0\n"
        );
    }

    #[test]
    fn test_row_diff_added_entry_one_plus_line() {
        let diff = row_diff(&[], &[entry("coffee", "1", "12")]);
        assert_eq!(diff, "+ coffee | 1 | 12, 0, 0, 0, 0, 0\n");
    }

    #[test]
    fn test_row_diff_unchanged_is_empty() {
        let e = entry("coffee", "1", "12");
        assert_eq!(
            row_diff(std::slice::from_ref(&e), std::slice::from_ref(&e)),
            ""
        );
    }

    #[test]
    fn test_row_diff_removed_entry_one_minus_line() {
        let diff = row_diff(&[entry("coffee", "1", "12")], &[]);
        assert_eq!(diff, "- coffee | 1 | 12, 0, 0, 0, 0, 0\n");
    }

    #[test]
    fn test_capture_prompt_prefers_positional_and_inline() {
        assert_eq!(
            capture_prompt(&["add".to_string(), "dinner".to_string()], None).unwrap(),
            Some("add dinner".to_string())
        );
        assert_eq!(
            capture_prompt(&[], Some("inline")).unwrap(),
            Some("inline".to_string())
        );
        assert_eq!(
            capture_prompt(&["both".to_string()], Some("inline")).unwrap(),
            Some("inline".to_string())
        );
    }

    #[test]
    fn test_resolve_settings_merges_config_env_and_flags() {
        let config: Config =
            toml::from_str("[ai]\napi_key = \"cfg\"\nmodel = \"m1\"\ntrace_requests = true\n")
                .unwrap();
        let flags = cli::AiFlags {
            api_key: Some("cli".to_string()),
            model: None,
            yes: false,
            trace_requests: false,
            trace_responses: true,
            prompt_arg: None,
        };
        std::env::set_var("INTAKE_AI_API_KEY", "env");
        std::env::set_var("INTAKE_AI_MODEL", "m2");
        let settings = resolve_settings(&config, &flags).unwrap();
        std::env::remove_var("INTAKE_AI_API_KEY");
        std::env::remove_var("INTAKE_AI_MODEL");
        assert_eq!(settings.api_key, "cli");
        assert_eq!(settings.model, "m2");
        assert!(settings.trace_requests);
        assert!(settings.trace_responses);
    }

    #[test]
    fn test_resolve_settings_defaults_without_config() {
        let config = Config::default();
        let flags = cli::AiFlags {
            api_key: None,
            model: None,
            yes: false,
            trace_requests: false,
            trace_responses: false,
            prompt_arg: None,
        };
        let settings = resolve_settings(&config, &flags).unwrap();
        assert_eq!(settings.model, intake_ai::settings::DEFAULT_MODEL);
        assert_eq!(settings.usda_api_key, None);
        assert!(!settings.trace_requests);
        assert!(!settings.trace_responses);
    }

    #[test]
    fn test_resolve_settings_usda_key_from_env() {
        let config = Config::default();
        let flags = cli::AiFlags {
            api_key: None,
            model: None,
            yes: false,
            trace_requests: false,
            trace_responses: false,
            prompt_arg: None,
        };
        std::env::set_var("INTAKE_AI_USDA_API_KEY", "usda-key");
        let settings = resolve_settings(&config, &flags).unwrap();
        std::env::remove_var("INTAKE_AI_USDA_API_KEY");
        assert_eq!(settings.usda_api_key.as_deref(), Some("usda-key"));
    }

    #[test]
    fn test_ai_log_happy_path_writes_day() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        write_day(log_dir.path(), date, vec![entry("coffee", "1", "12")]);
        let ops = "[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 2\n";
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(ops))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "add oatmeal",
            |w| Box::new(Scripted::with_writer(w, vec![ConfirmDecision::Accept])),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&out).contains("- coffee | 1 | 12, 0, 0, 0, 0, 0"));
        assert!(String::from_utf8_lossy(&out).contains("+ Oatmeal | 2 | 200, 10, 5, 4, 30, 0"));
        let loaded = log::load_day(log_dir.path(), date).unwrap().unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[1].title, "Oatmeal");
        assert_eq!(loaded.entries[1].servings, Servings::from_str("2").unwrap());
    }

    #[test]
    fn test_ai_log_accept_empty_ops_no_changes() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        write_day(log_dir.path(), date, vec![entry("coffee", "1", "12")]);
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text("ops = []\n"))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "no changes",
            |w| Box::new(Scripted::with_writer(w, vec![ConfirmDecision::Accept])),
        )
        .unwrap();
        assert!(
            String::from_utf8_lossy(&out).contains("No changes."),
            "no-op proposal must be shown before confirmation"
        );
        assert!(String::from_utf8_lossy(&out).contains("No changes"));
        let loaded = log::load_day(log_dir.path(), date).unwrap().unwrap();
        assert_eq!(loaded.entries.len(), 1);
    }

    #[test]
    fn test_ai_log_skip_present_auto_accepts_and_writes() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        write_day(log_dir.path(), date, vec![entry("coffee", "1", "12")]);
        let ops = "[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 1\n";
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(ops))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "add oatmeal",
            |_| Box::new(Scripted::skip_present(vec![ConfirmDecision::Reject])),
        )
        .unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(
            !text.contains("coffee | 1 | 12"),
            "proposal must not be rendered without present: {text}"
        );
        let loaded = log::load_day(log_dir.path(), date).unwrap().unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[1].title, "Oatmeal");
    }

    #[test]
    fn test_ai_log_reject_exits_ok_nothing_written() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(
            "[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 1\n",
        ))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "x",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Reject])),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&out).contains("Nothing written"));
        assert!(log::load_day(log_dir.path(), date).unwrap().is_none());
    }

    #[test]
    fn test_ai_log_cancelled_exits_ok_nothing_written() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(
            "[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 1\n",
        ))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "x",
            |_| Box::new(Scripted::results(vec![Err(ConfirmError::Cancelled)])),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&out).contains("Nothing written"));
    }

    #[test]
    fn test_ai_log_noop_goes_through_confirmation() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        write_day(log_dir.path(), date, vec![entry("coffee", "1", "12")]);
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text("ops = []\n"))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "no changes",
            |w| Box::new(Scripted::with_writer(w, vec![ConfirmDecision::Reject])),
        )
        .unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("No changes."),
            "no-op proposal was not shown: {text}"
        );
        assert!(
            !text.contains("coffee | 1 | 12"),
            "no-op proposal must not render the day rows: {text}"
        );
        assert!(text.contains("Nothing written"), "got: {text}");
        assert_eq!(
            log::load_day(log_dir.path(), date)
                .unwrap()
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn test_ai_log_noop_feedback_reruns_then_confirms() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        write_day(log_dir.path(), date, vec![entry("coffee", "1", "12")]);
        let backend = FakeBackend::new(vec![
            Ok(AssistantMessage::text("ops = []\n")),
            Ok(AssistantMessage::text("ops = []\n")),
        ]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "no changes",
            |w| {
                Box::new(Scripted::with_writer(
                    w,
                    vec![
                        ConfirmDecision::Feedback("try again".to_string()),
                        ConfirmDecision::Accept,
                    ],
                ))
            },
        )
        .unwrap();
        let text = String::from_utf8_lossy(&out);
        assert_eq!(text.matches("No changes.").count(), 2, "got: {text}");
        assert_eq!(text.matches("No changes").count(), 3, "got: {text}");
        assert_eq!(
            log::load_day(log_dir.path(), date)
                .unwrap()
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn test_ai_food_new_reject_exits_ok_nothing_written() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = FoodName::from_str("my-food").unwrap();
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(
            "title = \"X\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        ))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_food_new(&mut out, &env, dir.path(), &name, "make it", |_| {
            Box::new(Scripted::new(vec![ConfirmDecision::Reject]))
        })
        .unwrap();
        assert!(String::from_utf8_lossy(&out).contains("Nothing written"));
        assert!(!name.file_path(dir.path()).exists());
    }

    #[test]
    fn test_ai_log_exhausted_is_error() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        let backend = FakeBackend::new(vec![
            Ok(AssistantMessage::text("not toml at all")),
            Ok(AssistantMessage::text("not toml at all")),
            Ok(AssistantMessage::text("not toml at all")),
            Ok(AssistantMessage::text("not toml at all")),
        ]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        let err = cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "x",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Reject])),
        )
        .unwrap_err();
        assert!(err.to_string().contains("could not produce valid output"));
        assert!(err.to_string().contains("not toml at all"));
    }

    #[test]
    fn test_ai_log_backend_io_error_is_error() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        let backend = FakeBackend::new(vec![Err(LlmError::Timeout)]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        let err = cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "x",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Reject])),
        )
        .unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn test_ai_food_new_writes_food() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = FoodName::from_str("my-food").unwrap();
        let toml = "title = \"My Food\"\nservings = 2\n\n[[ingredients]]\nname = \"Chicken\"\nquantity = \"200g\"\ncalories = 330\nprotein_g = 46\nfiber_g = 0\nfat_g = 6\ncarbs_g = 0\nalcohol_g = 0\n";
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(toml))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_food_new(&mut out, &env, dir.path(), &name, "make it", |_| {
            Box::new(Scripted::new(vec![ConfirmDecision::Accept]))
        })
        .unwrap();
        assert!(name.file_path(dir.path()).exists());
        let loaded = food::load_food(&name.file_path(dir.path())).unwrap();
        assert_eq!(loaded.title, "My Food");
        assert_eq!(loaded.ingredients.len(), 1);
    }

    #[test]
    fn test_ai_food_new_parse_time_collision_errors() {
        let dir = foods_dir();
        let name = FoodName::from_str("oatmeal").unwrap();
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(
            "title = \"X\"\nservings = 1\n",
        ))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        let err = cmd_ai_food_new(&mut out, &env, dir.path(), &name, "x", |_| {
            Box::new(Scripted::new(vec![ConfirmDecision::Reject]))
        })
        .unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert!(
            err.to_string().contains("`ai food edit oatmeal`"),
            "got: {err}"
        );
    }

    #[test]
    fn test_ai_food_new_write_time_recheck_aborts() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = FoodName::from_str("my-food").unwrap();
        let backend = AppearingFileBackend {
            path: name.file_path(dir.path()),
            first: Mutex::new(Some(true)),
        };
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        let err = cmd_ai_food_new(&mut out, &env, dir.path(), &name, "make it", |_| {
            Box::new(Scripted::new(vec![ConfirmDecision::Accept]))
        })
        .unwrap_err();
        assert!(err.to_string().contains("already exists"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(name.file_path(dir.path())).unwrap(),
            "title = \"Sneaked In\"\nservings = 1\n\n[[ingredients]]\nname = \"X\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
            "the concurrent file must not be overwritten"
        );
    }

    struct AppearingFileBackend {
        path: std::path::PathBuf,
        first: Mutex<Option<bool>>,
    }

    impl LlmBackend for AppearingFileBackend {
        fn complete(
            &self,
            _messages: &[Message],
            _tools: &[&dyn Tool],
        ) -> Result<AssistantMessage, LlmError> {
            let mut first = self.first.lock().unwrap();
            if first.take().unwrap_or(false) {
                std::fs::write(
                    &self.path,
                    "title = \"Sneaked In\"\nservings = 1\n\n[[ingredients]]\nname = \"X\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
                )
                .unwrap();
            }
            Ok(AssistantMessage::text(
                "title = \"Proposed\"\nservings = 1\n\n[[ingredients]]\nname = \"Y\"\ncalories = 2\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
            ))
        }
    }

    #[test]
    fn test_ai_food_edit_happy_path_overwrites() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = FoodName::from_str("my-food").unwrap();
        let original = "title = \"My Food\"\nservings = 2\n\n[[ingredients]]\nname = \"Chicken\"\ncalories = 330\nprotein_g = 46\nfiber_g = 0\nfat_g = 6\ncarbs_g = 0\nalcohol_g = 0\n";
        std::fs::write(name.file_path(dir.path()), original).unwrap();
        let edited = "title = \"My Food v2\"\nservings = 4\n\n[[ingredients]]\nname = \"Chicken\"\ncalories = 660\nprotein_g = 92\nfiber_g = 0\nfat_g = 12\ncarbs_g = 0\nalcohol_g = 0\n";
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(edited))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_food_edit(&mut out, &env, dir.path(), &name, "double it", |_| {
            Box::new(Scripted::new(vec![ConfirmDecision::Accept]))
        })
        .unwrap();
        let loaded = food::load_food(&name.file_path(dir.path())).unwrap();
        assert_eq!(loaded.title, "My Food v2");
        assert_eq!(loaded.servings.get(), 4);
    }

    #[test]
    fn test_ai_food_edit_stale_file_aborts() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = FoodName::from_str("my-food").unwrap();
        let original = "title = \"My Food\"\nservings = 2\n\n[[ingredients]]\nname = \"Chicken\"\ncalories = 330\nprotein_g = 46\nfiber_g = 0\nfat_g = 6\ncarbs_g = 0\nalcohol_g = 0\n";
        std::fs::write(name.file_path(dir.path()), original).unwrap();
        let backend = ConcurrentEditBackend {
            path: name.file_path(dir.path()),
            first: Mutex::new(Some(true)),
        };
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        let err = cmd_ai_food_edit(&mut out, &env, dir.path(), &name, "double it", |_| {
            Box::new(Scripted::new(vec![ConfirmDecision::Accept]))
        })
        .unwrap_err();
        assert!(err.to_string().contains("changed since this proposal"));
    }

    struct ConcurrentEditBackend {
        path: std::path::PathBuf,
        first: Mutex<Option<bool>>,
    }

    impl LlmBackend for ConcurrentEditBackend {
        fn complete(
            &self,
            _messages: &[Message],
            _tools: &[&dyn Tool],
        ) -> Result<AssistantMessage, LlmError> {
            let mut first = self.first.lock().unwrap();
            if first.take().unwrap_or(false) {
                std::fs::write(
                    &self.path,
                    "title = \"Concurrently Changed\"\nservings = 1\n\n[[ingredients]]\nname = \"X\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
                )
                .unwrap();
            }
            Ok(AssistantMessage::text(
                "title = \"Proposed\"\nservings = 1\n\n[[ingredients]]\nname = \"Y\"\ncalories = 2\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
            ))
        }
    }

    #[test]
    fn test_ai_food_edit_no_changes_prints_and_skips_write() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = FoodName::from_str("my-food").unwrap();
        let original = "title = \"My Food\"\nservings = 2\n\n[[ingredients]]\nname = \"Chicken\"\ncalories = 330\nprotein_g = 46\nfiber_g = 0\nfat_g = 6\ncarbs_g = 0\nalcohol_g = 0\n";
        std::fs::write(name.file_path(dir.path()), original).unwrap();
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(original))]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        cmd_ai_food_edit(&mut out, &env, dir.path(), &name, "no change", |_| {
            Box::new(Scripted::skip_present(vec![ConfirmDecision::Accept]))
        })
        .unwrap();
        assert!(String::from_utf8_lossy(&out).contains("No changes"));
        let loaded = food::load_food(&name.file_path(dir.path())).unwrap();
        assert_eq!(loaded.title, "My Food");
    }

    #[test]
    fn test_ai_food_new_invalid_food_toml_retries_then_exhausts() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = FoodName::from_str("my-food").unwrap();
        let backend = FakeBackend::new(vec![
            Ok(AssistantMessage::text("title = \"X\"")),
            Ok(AssistantMessage::text("title = \"X\"")),
            Ok(AssistantMessage::text("title = \"X\"")),
            Ok(AssistantMessage::text("title = \"X\"")),
        ]);
        let mut out = Vec::new();
        let config = Config::default();
        let settings = settings();
        let env = AiEnv {
            settings: &settings,
            backend: &backend,
            config: &config,
        };
        let err = cmd_ai_food_new(&mut out, &env, dir.path(), &name, "x", |_| {
            Box::new(Scripted::new(vec![ConfirmDecision::Reject]))
        })
        .unwrap_err();
        assert!(err.to_string().contains("could not produce valid output"));
        assert!(!name.file_path(dir.path()).exists());
    }

    #[test]
    fn test_prompt_templates_contain_schema_tokens() {
        assert!(prompts::LOG.contains("add-adhoc"));
        assert!(prompts::LOG.contains("protein_g"));
        assert!(prompts::LOG.contains("kind = \"remove\""));
        assert!(prompts::FOOD_NEW.contains("protein_g"));
        assert!(prompts::FOOD_NEW.contains("servings"));
        assert!(prompts::FOOD_EDIT.contains("protein_g"));
        assert!(prompts::FOOD_EDIT.contains("servings"));
    }

    #[test]
    fn test_system_prompt_appends_context() {
        let text = system_prompt("BASE", None, "CONTEXT");
        assert!(text.starts_with("BASE"));
        assert!(text.ends_with("CONTEXT"));
        let text = system_prompt("BASE", Some("OVERRIDE"), "");
        assert_eq!(text, "OVERRIDE");
    }

    #[test]
    fn test_sample_foods_context_renders_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("simple.toml"),
            "title = \"Simple\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        )
        .unwrap();
        let ctx = sample_foods_context(dir.path()).unwrap();
        assert!(ctx.contains("Sample foods from your catalog"));
        assert!(ctx.contains("title = \"Simple\""));
        let empty = sample_foods_context(tempfile::TempDir::new().unwrap().path()).unwrap();
        assert!(empty.contains("Example food"), "got: {empty}");
        assert!(empty.contains("title = \"My Food\""), "got: {empty}");
        assert!(!empty.contains("Sample foods"), "got: {empty}");
    }
}
