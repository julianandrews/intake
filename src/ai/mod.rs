mod catalog;
pub(crate) mod cli;
mod confirm;
mod context;
mod food_lookup;
mod ops;
mod prompts;
pub(crate) mod settings;
mod spinner;
mod trace;
mod usda;
mod write;

use crate::amount::Calories;
use crate::commands;
use crate::config::Config;
use crate::editor;
use crate::food::{self, FoodName};
use crate::log;
use anyhow::{bail, Context, Result};
use intake_ai::confirm::Confirmer;
use intake_ai::llm::{LlmBackend, OpenAiCompatible};
use intake_ai::pipeline::{ResolveContext, ResolveError, Resolver};
use intake_ai::settings::Settings;
use intake_ai::tools::Tool;
use similar::{ChangeTag, TextDiff};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
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
            let date = commands::resolve_date(date.date, date.days_ago)?;
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
                |w| Box::new(confirm::AiConfirmer::new(w)),
                flags.yes,
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
                |w| Box::new(confirm::AiConfirmer::new(w)),
                flags.yes,
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
                |w| Box::new(confirm::AiConfirmer::new(w)),
                flags.yes,
            )
        }
    }
}

/// Whether the proposal was rendered to the user before the value was
/// confirmed: true when the interactive confirmation ran, false under
/// `--yes` auto-accept. After a successful write the commands branch on
/// this — when the proposal was shown, a one-line confirmation suffices;
/// when not, the affected table is reprinted as the only display.
fn proposal_presented(yes: bool) -> bool {
    !yes
}

fn write_nothing(writer: &mut impl Write) -> Result<()> {
    writeln!(writer, "Nothing written")?;
    Ok(())
}

fn handle_error(writer: &mut impl Write, err: ResolveError) -> Result<()> {
    match err {
        ResolveError::Rejected => write_nothing(writer),
        ResolveError::Exhausted {
            last_error,
            raw_output,
        } => bail!(
            "AI could not produce valid output: {last_error}\n\nLast model output:\n{raw_output}"
        ),
        ResolveError::Internal(e) => Err(anyhow::Error::msg(e)),
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

fn resolve_settings(config: &Config, flags: &cli::AiFlags) -> Result<Settings> {
    let ai = config.ai.as_ref();
    let model = resolve_value(
        ai.and_then(|ai| ai.model.clone()),
        "INTAKE_AI_MODEL",
        flags.model.clone(),
    )
    .ok_or_else(|| {
        anyhow::anyhow!("no model configured: set `[ai] model`, INTAKE_AI_MODEL, or --model")
    })?;
    let base_url = resolve_value(
        ai.and_then(|ai| ai.base_url.clone()),
        "INTAKE_AI_BASE_URL",
        flags.base_url.clone(),
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "no base_url configured: set `[ai] base_url`, INTAKE_AI_BASE_URL, or --base-url"
        )
    })?;
    let api_key = resolve_value(
        ai.and_then(|ai| ai.api_key.clone()),
        "INTAKE_AI_API_KEY",
        flags.api_key.clone(),
    );
    let mut settings = Settings::new(base_url, model, api_key);
    if let Some(max_retries) = ai.and_then(|ai| ai.max_retries) {
        settings.max_retries = max_retries;
    }
    if let Some(max_tool_calls) = ai.and_then(|ai| ai.max_tool_calls) {
        settings.max_tool_calls = max_tool_calls;
    }
    if let Some(timeout_secs) = ai.and_then(|ai| ai.timeout_secs) {
        settings.timeout_secs = timeout_secs;
    }
    settings.trace_requests =
        ai.is_some_and(|ai| ai.trace_requests.unwrap_or(false)) || flags.trace_requests;
    settings.trace_responses =
        ai.is_some_and(|ai| ai.trace_responses.unwrap_or(false)) || flags.trace_responses;
    Ok(settings)
}

fn resolve_value(
    from_config: Option<String>,
    env_name: &str,
    flag: Option<String>,
) -> Option<String> {
    flag.or_else(|| std::env::var(env_name).ok().filter(|v| !v.is_empty()))
        .or(from_config)
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
    settings: &'a Settings,
    backend: &'a dyn LlmBackend,
    config: &'a Config,
}

/// Resolved settings plus the backend they build, owning both so an [`AiEnv`]
/// can borrow them for the duration of a command. Constructed per command
/// from the config file → env var → CLI flag resolution in
/// [`resolve_settings`].
struct AiSession {
    settings: Settings,
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

fn usda_key(config: &Config) -> String {
    let mut key = config
        .ai
        .as_ref()
        .and_then(|ai| ai.usda_api_key.as_deref())
        .unwrap_or("")
        .to_string();
    if let Ok(value) = std::env::var("INTAKE_AI_USDA_API_KEY") {
        if !value.is_empty() {
            key = value;
        }
    }
    key
}

fn usda_tools(config: &Config) -> usda::UsdaSearch {
    let timeout = Duration::from_secs(
        config
            .ai
            .as_ref()
            .and_then(|ai| ai.usda_timeout_secs)
            .unwrap_or(settings::DEFAULT_USDA_TIMEOUT_SECS),
    );
    usda::UsdaSearch::new(&usda_key(config), timeout)
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
    let mut before_text = before
        .iter()
        .map(context::entry_line)
        .collect::<Vec<_>>()
        .join("\n");
    let mut after_text = after
        .iter()
        .map(context::entry_line)
        .collect::<Vec<_>>()
        .join("\n");
    // `similar` counts each line's trailing newline as part of the line, so
    // an unterminated final line differs from a terminated one. Both inputs
    // must therefore end in a newline: otherwise appending an entry after
    // the last line would diff the untouched previous line as deleted and
    // re-inserted instead of showing a single insertion. A side with no
    // entries stays empty so it doesn't contribute a phantom blank line.
    if !before_text.is_empty() {
        before_text.push('\n');
    }
    if !after_text.is_empty() {
        after_text.push('\n');
    }
    let diff = TextDiff::from_lines(&before_text, &after_text);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => continue,
        };
        // Lines are newline-terminated tokens; strip exactly the newline so
        // trailing whitespace inside a line survives untouched.
        out.push_str(&format!(
            "{sign} {}\n",
            change.value().trim_end_matches('\n')
        ));
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
    yes: bool,
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
        trace_observer: Some(Arc::new(trace::TraceRenderer::new(
            status.writer(),
            crate::display::color_enabled(),
        ))),
        auto_accept: yes,
    };
    let mut resolver = Resolver::new(&ctx, &mut confirmer, job.system);
    resolver.resolve(job.prompt, job.parse, job.present)
}

#[allow(clippy::too_many_arguments)]
fn cmd_ai_log(
    writer: &mut impl Write,
    env: &AiEnv<'_>,
    foods_dir: &Path,
    log_dir: &Path,
    date: chrono::NaiveDate,
    prompt: &str,
    make_confirmer: impl FnOnce(&mut dyn Write) -> Box<dyn Confirmer + '_>,
    yes: bool,
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
    let usda_search = usda_tools(config);
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

    let (outcome, presented) = {
        let confirmer = make_confirmer(writer);
        let presented = proposal_presented(yes);
        let tools: [&dyn Tool; 2] = [&food_lookup, &usda_search];
        let outcome = resolve_session(
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
            yes,
        );
        (outcome, presented)
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
            if presented {
                writeln!(writer, "Logged to {date}")?;
            } else {
                writeln!(writer)?;
                crate::commands::log::cmd_day(writer, log_dir, date, config)?;
            }
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
    yes: bool,
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
    let usda_search = usda_tools(config);
    let status = spinner::StatusLine::new(settings);
    let columns = config.columns()?;

    let parse =
        |s: &str| -> Result<food::Food, String> { toml::from_str(s).map_err(|e| e.to_string()) };
    let present = |f: &food::Food| -> String {
        crate::commands::food::render_food(f, &columns)
            .unwrap_or_else(|e| format!("(unavailable: {e})"))
    };

    let (outcome, presented) = {
        let confirmer = make_confirmer(writer);
        let presented = proposal_presented(yes);
        let tools: [&dyn Tool; 1] = [&usda_search];
        let outcome = resolve_session(
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
            yes,
        );
        (outcome, presented)
    };

    match outcome {
        Ok(new_food) => {
            food::create_food(foods_dir, name, &new_food)?;
            if presented {
                writeln!(writer, "Wrote {}", path.display())?;
            } else {
                writeln!(writer)?;
                write!(
                    writer,
                    "{}",
                    crate::commands::food::render_food(&new_food, &columns)?
                )?;
            }
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
    yes: bool,
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
    let usda_search = usda_tools(config);
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

    let (outcome, presented) = {
        let confirmer = make_confirmer(writer);
        let presented = proposal_presented(yes);
        let tools: [&dyn Tool; 1] = [&usda_search];
        let outcome = resolve_session(
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
            yes,
        );
        (outcome, presented)
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
            if presented {
                writeln!(writer, "Wrote {}", path.display())?;
            } else {
                writeln!(writer)?;
                write!(
                    writer,
                    "{}",
                    crate::commands::food::render_food(&new_food, &columns)?
                )?;
            }
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
    use intake_ai::confirm::ConfirmDecision;
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
        queue: Mutex<Vec<Result<ConfirmDecision, Box<dyn std::error::Error + Send + Sync>>>>,
        out: Option<&'a mut dyn Write>,
    }

    impl<'a> Scripted<'a> {
        fn new(decisions: Vec<ConfirmDecision>) -> Scripted<'a> {
            Scripted {
                queue: Mutex::new(decisions.into_iter().map(Ok).collect()),
                out: None,
            }
        }

        fn results(
            results: Vec<Result<ConfirmDecision, Box<dyn std::error::Error + Send + Sync>>>,
        ) -> Scripted<'a> {
            Scripted {
                queue: Mutex::new(results),
                out: None,
            }
        }

        fn with_writer(writer: &'a mut dyn Write, decisions: Vec<ConfirmDecision>) -> Scripted<'a> {
            Scripted {
                queue: Mutex::new(decisions.into_iter().map(Ok).collect()),
                out: Some(writer),
            }
        }
    }

    impl Confirmer for Scripted<'_> {
        fn confirm(
            &mut self,
            rendered: &str,
        ) -> Result<ConfirmDecision, Box<dyn std::error::Error + Send + Sync>> {
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
    }

    fn settings() -> Settings {
        Settings::new("http://test/v1", "m", None)
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
    fn test_row_diff_appended_entry_keeps_previous_line_untouched() {
        let diff = row_diff(
            &[entry("protein shake", "1", "275")],
            &[
                entry("protein shake", "1", "275"),
                entry("apple", "1", "52"),
            ],
        );
        assert_eq!(diff, "+ apple | 1 | 52, 0, 0, 0, 0, 0\n");
    }

    #[test]
    fn test_row_diff_removed_last_entry_keeps_previous_line_untouched() {
        let diff = row_diff(
            &[
                entry("protein shake", "1", "275"),
                entry("apple", "1", "52"),
            ],
            &[entry("protein shake", "1", "275")],
        );
        assert_eq!(diff, "- apple | 1 | 52, 0, 0, 0, 0, 0\n");
    }

    #[test]
    fn test_row_diff_middle_edit_keeps_unchanged_last_line() {
        let mut shake = entry("protein shake", "1", "275");
        shake.calories = crate::amount::Calories::from_str("276").unwrap();
        let diff = row_diff(
            &[
                entry("protein shake", "1", "275"),
                entry("apple", "1", "52"),
            ],
            &[shake, entry("apple", "1", "52")],
        );
        assert_eq!(
            diff,
            "- protein shake | 1 | 275, 0, 0, 0, 0, 0\n+ protein shake | 1 | 276, 0, 0, 0, 0, 0\n"
        );
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
        let config: Config = toml::from_str(
            "[ai]\napi_key = \"cfg\"\nmodel = \"m1\"\nbase_url = \"http://cfg/v1\"\ntrace_requests = true\n",
        )
        .unwrap();
        let flags = cli::AiFlags {
            api_key: Some("cli".to_string()),
            model: None,
            base_url: None,
            yes: false,
            trace_requests: false,
            trace_responses: true,
            prompt_arg: None,
        };
        std::env::set_var("INTAKE_AI_API_KEY", "env");
        std::env::set_var("INTAKE_AI_MODEL", "m2");
        std::env::set_var("INTAKE_AI_BASE_URL", "http://env/v1");
        let settings = resolve_settings(&config, &flags).unwrap();
        std::env::remove_var("INTAKE_AI_API_KEY");
        std::env::remove_var("INTAKE_AI_MODEL");
        std::env::remove_var("INTAKE_AI_BASE_URL");
        assert_eq!(settings.api_key.as_deref(), Some("cli"));
        assert_eq!(settings.model, "m2");
        assert_eq!(settings.base_url, "http://env/v1");
        assert!(settings.trace_requests);
        assert!(settings.trace_responses);
    }

    #[test]
    fn test_resolve_settings_base_url_flag_wins() {
        let config: Config =
            toml::from_str("[ai]\nmodel = \"m\"\nbase_url = \"http://cfg/v1\"\n").unwrap();
        let flags = cli::AiFlags {
            api_key: None,
            model: None,
            base_url: Some("http://cli/v1".to_string()),
            yes: false,
            trace_requests: false,
            trace_responses: false,
            prompt_arg: None,
        };
        std::env::set_var("INTAKE_AI_BASE_URL", "http://env/v1");
        let settings = resolve_settings(&config, &flags).unwrap();
        std::env::remove_var("INTAKE_AI_BASE_URL");
        assert_eq!(settings.base_url, "http://cli/v1");
        assert_eq!(settings.model, "m");
    }

    #[test]
    fn test_resolve_settings_requires_model_and_base_url() {
        let config = Config::default();
        let flags = cli::AiFlags {
            api_key: None,
            model: None,
            base_url: None,
            yes: false,
            trace_requests: false,
            trace_responses: false,
            prompt_arg: None,
        };
        let err = resolve_settings(&config, &flags).unwrap_err().to_string();
        assert!(err.contains("no model configured"), "got: {err}");
    }

    #[test]
    fn test_resolve_settings_operational_defaults() {
        let config = Config::default();
        let flags = cli::AiFlags {
            api_key: None,
            model: Some("m".to_string()),
            base_url: Some("http://x/v1".to_string()),
            yes: false,
            trace_requests: false,
            trace_responses: false,
            prompt_arg: None,
        };
        let settings = resolve_settings(&config, &flags).unwrap();
        assert_eq!(settings.model, "m");
        assert_eq!(settings.base_url, "http://x/v1");
        assert_eq!(settings.api_key, None);
        assert_eq!(settings.max_retries, 3);
        assert_eq!(settings.max_tool_calls, 20);
        assert_eq!(settings.timeout_secs, 60);
        assert!(!settings.trace_requests);
        assert!(!settings.trace_responses);
    }

    #[test]
    fn test_usda_key_from_config() {
        let config: Config = toml::from_str("[ai]\nusda_api_key = \"k\"\n").unwrap();
        assert_eq!(usda_key(&config), "k");
        assert_eq!(usda_key(&Config::default()), "");
    }

    #[test]
    fn test_usda_key_env_overrides_config() {
        let config: Config = toml::from_str("[ai]\nusda_api_key = \"k\"\n").unwrap();
        std::env::set_var("INTAKE_AI_USDA_API_KEY", "usda-key");
        let key = usda_key(&config);
        std::env::remove_var("INTAKE_AI_USDA_API_KEY");
        assert_eq!(key, "usda-key");
    }

    #[test]
    fn test_usda_key_from_env() {
        let config = Config::default();
        std::env::set_var("INTAKE_AI_USDA_API_KEY", "usda-key");
        let key = usda_key(&config);
        std::env::remove_var("INTAKE_AI_USDA_API_KEY");
        assert_eq!(key, "usda-key");
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
            false,
        )
        .unwrap();
        let text = String::from_utf8_lossy(&out);
        // The confirmation must present exactly one insertion, leading the
        // output: appending after the last entry must not diff the untouched
        // "coffee" line as deleted and re-inserted.
        assert!(text.starts_with("+ Oatmeal | 2 | 200, 10, 5, 4, 30, 0\n\n"));
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
            false,
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
    fn test_ai_log_yes_auto_accepts_and_writes() {
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
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Reject])),
            true,
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
            false,
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&out).contains("Nothing written"));
        assert!(log::load_day(log_dir.path(), date).unwrap().is_none());
    }

    #[test]
    fn test_ai_log_confirmer_error_is_error() {
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
        let err = cmd_ai_log(
            &mut out,
            &env,
            dir.path(),
            log_dir.path(),
            date,
            "x",
            |_| {
                Box::new(Scripted::results(vec![Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "confirmation broke",
                )))]))
            },
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("confirmation broke"));
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
            false,
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
            false,
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
        cmd_ai_food_new(
            &mut out,
            &env,
            dir.path(),
            &name,
            "make it",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Reject])),
            false,
        )
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
            false,
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
            false,
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
        cmd_ai_food_new(
            &mut out,
            &env,
            dir.path(),
            &name,
            "make it",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Accept])),
            false,
        )
        .unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains(&format!("Wrote {}", name.file_path(dir.path()).display())),
            "confirmation line missing: {text}"
        );
        assert!(
            !text.contains("My Food"),
            "table must not be reprinted after presentation: {text}"
        );
        assert!(name.file_path(dir.path()).exists());
        let loaded = food::load_food(&name.file_path(dir.path())).unwrap();
        assert_eq!(loaded.title, "My Food");
        assert_eq!(loaded.ingredients.len(), 1);
    }

    #[test]
    fn test_ai_food_new_yes_renders_table() {
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
        cmd_ai_food_new(
            &mut out,
            &env,
            dir.path(),
            &name,
            "make it",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Accept])),
            true,
        )
        .unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("My Food"),
            "table must be reprinted without present: {text}"
        );
        assert!(!text.contains("Wrote"), "got: {text}");
        assert!(name.file_path(dir.path()).exists());
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
        let err = cmd_ai_food_new(
            &mut out,
            &env,
            dir.path(),
            &name,
            "x",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Reject])),
            false,
        )
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
        let err = cmd_ai_food_new(
            &mut out,
            &env,
            dir.path(),
            &name,
            "make it",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Accept])),
            false,
        )
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
        cmd_ai_food_edit(
            &mut out,
            &env,
            dir.path(),
            &name,
            "double it",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Accept])),
            false,
        )
        .unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains(&format!("Wrote {}", name.file_path(dir.path()).display())),
            "confirmation line missing: {text}"
        );
        assert!(
            !text.contains("My Food v2"),
            "table must not be reprinted after presentation: {text}"
        );
        let loaded = food::load_food(&name.file_path(dir.path())).unwrap();
        assert_eq!(loaded.title, "My Food v2");
        assert_eq!(loaded.servings.get(), 4);
    }

    #[test]
    fn test_ai_food_edit_yes_renders_table() {
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
        cmd_ai_food_edit(
            &mut out,
            &env,
            dir.path(),
            &name,
            "double it",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Accept])),
            true,
        )
        .unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("My Food v2"),
            "table must be reprinted without present: {text}"
        );
        assert!(!text.contains("Wrote"), "got: {text}");
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
        let err = cmd_ai_food_edit(
            &mut out,
            &env,
            dir.path(),
            &name,
            "double it",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Accept])),
            false,
        )
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
        cmd_ai_food_edit(
            &mut out,
            &env,
            dir.path(),
            &name,
            "no change",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Accept])),
            true,
        )
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
        let err = cmd_ai_food_new(
            &mut out,
            &env,
            dir.path(),
            &name,
            "x",
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Reject])),
            false,
        )
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
    fn test_query_style_block_spliced_into_every_prompt() {
        for (name, text) in [
            ("log", prompts::LOG),
            ("food_new", prompts::FOOD_NEW),
            ("food_edit", prompts::FOOD_EDIT),
        ] {
            assert!(text.contains("identifying words"), "{name}");
            assert!(text.contains("never quantities, units, or"), "{name}");
            assert!(text.contains("scale its per-100g values"), "{name}");
            assert!(text.contains("round trips are expensive"), "{name}");
        }
        assert!(prompts::LOG.contains("strip portion suffixes"));
        assert!(!prompts::FOOD_NEW.contains("strip portion suffixes"));
        assert!(!prompts::FOOD_EDIT.contains("strip portion suffixes"));
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
