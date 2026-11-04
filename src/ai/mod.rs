mod catalog;
pub(crate) mod cli;
mod confirm;
mod context;
mod food_lookup;
mod ops;
mod prompts;
mod proposal;
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

/// The User-Agent every intake request identifies itself with, for both
/// the LLM backend and the USDA tool. Compile-time; no user data.
pub(crate) const USER_AGENT: &str = concat!("intake/", env!("CARGO_PKG_VERSION"));

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
    let api_key = resolve_secret(
        ai.and_then(|ai| ai.api_key.as_ref()),
        "api_key",
        "INTAKE_AI_API_KEY",
        flags.api_key.clone(),
        config.config_dir.as_deref(),
    )?;
    let mut settings = Settings::new(base_url, model, api_key);
    settings.user_agent = USER_AGENT.to_string();
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
    if let Some(session_header) = ai.and_then(|ai| ai.session_header.clone()) {
        settings.session_header = Some(session_header);
    }
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

/// Resolve a secret with the same flag → env → config precedence as
/// `resolve_value`, but only touch the config source (file or command) when
/// neither a flag nor a non-empty environment variable supplied the value.
/// This keeps a configured command from running when it isn't needed and
/// avoids failing on an unreadable file that would be overridden anyway.
fn resolve_secret(
    from_config: Option<&settings::Secret>,
    label: &str,
    env_name: &str,
    flag: Option<String>,
    base_dir: Option<&Path>,
) -> Result<Option<String>> {
    if let Some(flag) = flag {
        return Ok(Some(flag));
    }
    if let Ok(value) = std::env::var(env_name) {
        if !value.is_empty() {
            return Ok(Some(value));
        }
    }
    from_config
        .map(|secret| secret.resolve(label, base_dir))
        .transpose()
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

fn usda_key(config: &Config) -> Result<String> {
    let key = resolve_secret(
        config.ai.as_ref().and_then(|ai| ai.usda_api_key.as_ref()),
        "usda_api_key",
        "INTAKE_AI_USDA_API_KEY",
        None,
        config.config_dir.as_deref(),
    )?;
    Ok(key.unwrap_or_default())
}

fn usda_tools(config: &Config) -> Result<usda::UsdaSearch> {
    let timeout = Duration::from_secs(
        config
            .ai
            .as_ref()
            .and_then(|ai| ai.usda_timeout_secs)
            .unwrap_or(settings::DEFAULT_USDA_TIMEOUT_SECS),
    );
    Ok(usda::UsdaSearch::new(&usda_key(config)?, timeout))
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
    let usda_search = usda_tools(config)?;
    let status = spinner::StatusLine::new(settings);
    let warn = |msg: &str| status.warn(msg);
    let food_lookup = food_lookup::FoodLookup::new(foods_dir).with_warn(&warn);

    let base = original.clone().unwrap_or(log::DayLog {
        entries: Vec::new(),
        exercise_calories: Calories::ZERO,
        weights: Vec::new(),
        complete: false,
    });
    let parse = |s: &str| -> Result<proposal::Proposal<write::AppliedDay>, String> {
        let day_ops: ops::DayLogOps = toml::from_str(s).map_err(|e| e.to_string())?;
        let add_ops = day_ops
            .ops
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    ops::DayLogOp::AddFood { .. } | ops::DayLogOp::AddAdhoc { .. }
                )
            })
            .count();
        let day = ops::apply_ops(&base, &day_ops.ops, foods_dir)?;
        Ok(proposal::Proposal::new(
            write::AppliedDay { day, add_ops },
            day_ops.message,
        ))
    };
    let present = |proposal: &proposal::Proposal<write::AppliedDay>| -> String {
        let applied = &proposal.value;
        let diff = row_diff(
            original
                .as_ref()
                .map(|d| d.entries.as_slice())
                .unwrap_or(&[]),
            &applied.day.entries,
        );
        let (body, table_width) = if diff.is_empty() {
            (confirm::NO_CHANGES_PROPOSAL.to_string(), None)
        } else {
            match crate::commands::log::render_day_and_width(&applied.day, date, config) {
                Ok((table, width)) => (format!("{diff}\n\n{table}"), Some(width)),
                Err(e) => (format!("{diff}\n\n(day table unavailable: {e})"), None),
            }
        };
        proposal.render_with_table_width(body, table_width)
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
        Ok(proposal) => {
            let applied = proposal.value;
            let changed = match &original {
                Some(d) => d != &applied.day,
                None => !log::day_is_empty(&applied.day),
            };
            if !changed {
                writeln!(writer, "No changes")?;
                return Ok(());
            }
            // Stamp the added entries at write time, after confirmation, so
            // the stamp records when the write landed. `write_timestamps`
            // off leaves the additions untimed; `replace` ops keep their
            // rows' original timestamps.
            let applied = if config.write_timestamps() {
                write::stamp_added_entries(applied, log::Timestamp::now())
            } else {
                applied
            };
            write::write_day_checked(log_dir, date, original.as_ref(), applied.day)?;
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
    let usda_search = usda_tools(config)?;
    let status = spinner::StatusLine::new(settings);
    let columns = config.columns()?;

    let parse =
        |s: &str| -> Result<proposal::Proposal<food::Food>, String> { proposal::parse_food(s) };
    let present = |proposal: &proposal::Proposal<food::Food>| -> String {
        let (body, table_width) =
            match crate::commands::food::render_food_and_width(&proposal.value, &columns) {
                Ok((text, width)) => (text, Some(width)),
                Err(e) => (format!("(unavailable: {e})"), None),
            };
        proposal.render_with_table_width(body, table_width)
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
        Ok(proposal) => {
            let new_food = proposal.value;
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
    let usda_search = usda_tools(config)?;
    let status = spinner::StatusLine::new(settings);
    let columns = config.columns()?;

    let parse =
        |s: &str| -> Result<proposal::Proposal<food::Food>, String> { proposal::parse_food(s) };
    let present = |proposal: &proposal::Proposal<food::Food>| -> String {
        let before = match crate::commands::food::render_food_and_width(&original, &columns) {
            Ok((text, width)) => (text, Some(width)),
            Err(e) => (format!("(unavailable: {e})"), None),
        };
        let after = match crate::commands::food::render_food_and_width(&proposal.value, &columns) {
            Ok((text, width)) => (text, Some(width)),
            Err(e) => (format!("(unavailable: {e})"), None),
        };
        let table_width = match (before.1, after.1) {
            (Some(before), Some(after)) => Some(before.max(after)),
            (Some(width), None) | (None, Some(width)) => Some(width),
            (None, None) => None,
        };
        let body = format!("Before:\n{}\nAfter:\n{}", before.0, after.0);
        proposal.render_with_table_width(body, table_width)
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
        Ok(proposal) => {
            let new_food = proposal.value;
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

    /// Serializes tests that read or write the `INTAKE_AI_*` environment
    /// variables: cargo runs tests in parallel threads within one process, so
    /// one test's `std::env::set_var` / `remove_var` would otherwise race with
    /// another test's `resolve_settings` / `usda_key` reads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
            timestamp: None,
        }
    }

    fn write_day(dir: &Path, date: chrono::NaiveDate, entries: Vec<log::LogEntry>) {
        let day = log::DayLog {
            entries,
            exercise_calories: crate::amount::Calories::ZERO,
            weights: Vec::new(),
            complete: false,
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
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        assert_eq!(
            settings.user_agent,
            format!("intake/{}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(settings.max_retries, 3);
        assert_eq!(settings.max_tool_calls, 20);
        assert_eq!(settings.timeout_secs, 60);
        assert!(!settings.trace_requests);
        assert!(!settings.trace_responses);
        assert_eq!(settings.session_header, None);
    }

    #[test]
    fn test_resolve_settings_session_header_from_config() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config: Config =
            toml::from_str("[ai]\nmodel = \"m\"\nbase_url = \"http://x/v1\"\nsession_header = \"x-opencode-session\"\n")
                .unwrap();
        let flags = cli::AiFlags {
            api_key: None,
            model: None,
            base_url: None,
            yes: false,
            trace_requests: false,
            trace_responses: false,
            prompt_arg: None,
        };
        let settings = resolve_settings(&config, &flags).unwrap();
        assert_eq!(
            settings.session_header.as_deref(),
            Some("x-opencode-session")
        );
    }

    #[test]
    fn test_usda_key_from_config() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config: Config = toml::from_str("[ai]\nusda_api_key = \"k\"\n").unwrap();
        assert_eq!(usda_key(&config).unwrap(), "k");
        assert_eq!(usda_key(&Config::default()).unwrap(), "");
    }

    #[test]
    fn test_usda_key_env_overrides_config() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config: Config = toml::from_str("[ai]\nusda_api_key = \"k\"\n").unwrap();
        std::env::set_var("INTAKE_AI_USDA_API_KEY", "usda-key");
        let key = usda_key(&config).unwrap();
        std::env::remove_var("INTAKE_AI_USDA_API_KEY");
        assert_eq!(key, "usda-key");
    }

    #[test]
    fn test_usda_key_from_env() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config = Config::default();
        std::env::set_var("INTAKE_AI_USDA_API_KEY", "usda-key");
        let key = usda_key(&config).unwrap();
        std::env::remove_var("INTAKE_AI_USDA_API_KEY");
        assert_eq!(key, "usda-key");
    }

    #[test]
    fn test_usda_key_from_file() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("INTAKE_AI_USDA_API_KEY");
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usda-key");
        std::fs::write(&path, "file-key\n").unwrap();
        let config: Config = toml::from_str(&format!(
            "[ai]\nusda_api_key = {{ file = {:?} }}\n",
            path.to_string_lossy()
        ))
        .unwrap();
        assert_eq!(usda_key(&config).unwrap(), "file-key");
    }

    #[test]
    fn test_usda_key_broken_source_errors() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("INTAKE_AI_USDA_API_KEY");
        let config: Config =
            toml::from_str("[ai]\nusda_api_key = { file = \"/definitely/missing\" }\n").unwrap();
        let err = usda_key(&config).unwrap_err().to_string();
        assert!(
            err.contains("failed to read usda_api_key file"),
            "got: {err}"
        );
    }

    #[test]
    fn test_resolve_secret_flag_wins_without_resolving_config() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("INTAKE_AI_API_KEY");
        let config: Config = toml::from_str(
            "[ai]\napi_key = { file = \"/definitely/missing\" }\nmodel = \"m\"\nbase_url = \"http://x/v1\"\n",
        )
        .unwrap();
        let flags = cli::AiFlags {
            api_key: Some("cli".to_string()),
            model: None,
            base_url: None,
            yes: false,
            trace_requests: false,
            trace_responses: false,
            prompt_arg: None,
        };
        let settings = resolve_settings(&config, &flags).unwrap();
        assert_eq!(settings.api_key.as_deref(), Some("cli"));
    }

    #[test]
    fn test_resolve_secret_env_skips_broken_config_source() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config: Config = toml::from_str(
            "[ai]\napi_key = { command = \"exit 1\" }\nmodel = \"m\"\nbase_url = \"http://x/v1\"\n",
        )
        .unwrap();
        let flags = cli::AiFlags {
            api_key: None,
            model: None,
            base_url: None,
            yes: false,
            trace_requests: false,
            trace_responses: false,
            prompt_arg: None,
        };
        std::env::set_var("INTAKE_AI_API_KEY", "env");
        let settings = resolve_settings(&config, &flags).unwrap();
        std::env::remove_var("INTAKE_AI_API_KEY");
        assert_eq!(settings.api_key.as_deref(), Some("env"));
    }

    #[test]
    fn test_resolve_settings_api_key_from_file_config() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("INTAKE_AI_API_KEY");
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "file-key\n").unwrap();
        let config: Config = toml::from_str(&format!(
            "[ai]\napi_key = {{ file = {:?} }}\nmodel = \"m\"\nbase_url = \"http://x/v1\"\n",
            path.to_string_lossy()
        ))
        .unwrap();
        let flags = cli::AiFlags {
            api_key: None,
            model: None,
            base_url: None,
            yes: false,
            trace_requests: false,
            trace_responses: false,
            prompt_arg: None,
        };
        let settings = resolve_settings(&config, &flags).unwrap();
        assert_eq!(settings.api_key.as_deref(), Some("file-key"));
    }

    #[test]
    fn test_resolve_settings_broken_api_key_source_errors() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("INTAKE_AI_API_KEY");
        let config: Config = toml::from_str(
            "[ai]\napi_key = { command = \"exit 1\" }\nmodel = \"m\"\nbase_url = \"http://x/v1\"\n",
        )
        .unwrap();
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
        assert!(err.contains("api_key command failed"), "got: {err}");
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
        // The pre-existing entry keeps no timestamp; the added entry is
        // stamped at write time (the default `write_timestamps` is on).
        assert_eq!(loaded.entries[0].timestamp, None);
        assert!(loaded.entries[1].timestamp.is_some());
    }

    #[test]
    fn test_ai_log_write_timestamps_off_leaves_additions_untimed() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        write_day(log_dir.path(), date, vec![entry("coffee", "1", "12")]);
        let ops = "[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 2\n";
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(ops))]);
        let mut out = Vec::new();
        let config: Config = toml::from_str("write_timestamps = false\n").unwrap();
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
            |_| Box::new(Scripted::new(vec![ConfirmDecision::Accept])),
            true,
        )
        .unwrap();
        let loaded = log::load_day(log_dir.path(), date).unwrap().unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].timestamp, None);
        assert_eq!(
            loaded.entries[1].timestamp, None,
            "write_timestamps = false must leave ai log additions untimed"
        );
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
    fn test_ai_log_renders_message_with_proposal() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        write_day(log_dir.path(), date, vec![entry("coffee", "1", "12")]);
        let ops = "message = \"Assumed the oatmeal was plain.\"\n\n[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 1\n";
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
        assert!(
            text.contains("Note: Assumed the oatmeal was plain."),
            "message block missing: {text}"
        );
        assert!(
            text.find("Note:").unwrap() > text.find("Total").unwrap(),
            "message must follow the proposal body: {text}"
        );
        let day = std::fs::read_to_string(log_dir.path().join("2026-08-09.toml")).unwrap();
        assert!(
            !day.contains("Assumed the oatmeal was plain."),
            "message must not be written: {day}"
        );
    }

    #[test]
    fn test_ai_log_message_wraps_to_table_width() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        write_day(log_dir.path(), date, vec![entry("coffee", "1", "12")]);
        let response = "message = \"Assumed the oatmeal was plain and made with water rather than milk, and used the same serving size as the previous entry.\"\n\n[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 1\n";
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(response))]);
        let mut out = Vec::new();
        let config: Config = toml::from_str("show_columns = []\n").unwrap();
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
        let table_width = text
            .lines()
            .find(|line| line.contains("---"))
            .map(crate::display::visible_width)
            .expect("table separator");
        let note = &text[text.find("Note:").expect("message")..];
        let note_lines: Vec<&str> = note.lines().take_while(|line| !line.is_empty()).collect();
        assert!(note_lines.len() > 1, "message should wrap: {text}");
        for line in note_lines {
            assert!(
                crate::display::visible_width(line) <= table_width,
                "message wider than table ({table_width}): {line:?}"
            );
        }
    }

    #[test]
    fn test_ai_log_noop_keeps_message() {
        let dir = foods_dir();
        let log_dir = tempfile::TempDir::new().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
        write_day(log_dir.path(), date, vec![entry("coffee", "1", "12")]);
        let response = "message = \"Should the latte be whole milk?\"\nops = []\n";
        let backend = FakeBackend::new(vec![Ok(AssistantMessage::text(response))]);
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
        assert!(text.contains("No changes."), "got: {text}");
        assert!(
            text.contains("Note: Should the latte be whole milk?"),
            "no-op message missing: {text}"
        );
        assert!(
            text.find("No changes.").unwrap() < text.find("Note:").unwrap(),
            "message must follow the proposal body: {text}"
        );
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
    fn test_ai_food_new_renders_message_without_persisting_it() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = FoodName::from_str("my-food").unwrap();
        let toml = "message = \"Used chicken breast.\"\ntitle = \"My Food\"\nservings = 1\n\n[[ingredients]]\nname = \"Chicken\"\ncalories = 165\nprotein_g = 31\nfiber_g = 0\nfat_g = 3.6\ncarbs_g = 0\nalcohol_g = 0\n";
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
            |w| Box::new(Scripted::with_writer(w, vec![ConfirmDecision::Accept])),
            false,
        )
        .unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("Note: Used chicken breast."),
            "message block missing: {text}"
        );
        assert!(
            text.find("Note:").unwrap() < text.find("Wrote").unwrap(),
            "message must precede the write confirmation: {text}"
        );
        let saved = std::fs::read_to_string(name.file_path(dir.path())).unwrap();
        assert!(
            !saved.contains("message"),
            "message must not be written to the food file: {saved}"
        );
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
            assert!(
                text.contains("quantities, units, or portion words"),
                "{name}"
            );
            assert!(text.contains("scale its per-100g values"), "{name}");
            assert!(text.contains("trips are expensive"), "{name}");
            assert!(text.contains("only when you need nutrition data"), "{name}");
            assert!(!text.contains("Query `usda_search` with"), "{name}");
        }
        assert!(prompts::LOG.contains("strip portion suffixes"));
        assert!(!prompts::FOOD_NEW.contains("strip portion suffixes"));
        assert!(!prompts::FOOD_EDIT.contains("strip portion suffixes"));
    }

    #[test]
    fn test_prompt_templates_include_message_rules() {
        for (name, text) in [
            ("log", prompts::LOG),
            ("food_new", prompts::FOOD_NEW),
            ("food_edit", prompts::FOOD_EDIT),
        ] {
            assert!(text.contains("optional top-level `message`"), "{name}");
            assert!(text.contains("one or two"), "{name}");
            assert!(text.contains("Most proposals need no message"), "{name}");
        }
        assert!(prompts::LOG.contains("history digest already has a matching entry"));
        assert!(prompts::LOG.contains("use remove or replace on"));
        assert!(prompts::LOG.contains("Prefer a `food_lookup` match over a fresh USDA lookup"));
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
