# Design Doc: AI Commands for intake

Status: Drafting

## Overview

Add AI-powered commands to the `intake` CLI: generate foods, and edit
existing log days and foods. All commands share one
pipeline: capture a user prompt (via a `[prompt...]` positional, `--prompt`,
or `$EDITOR`), send it to an LLM
along with a base prompt, let the model use function-calling tools
(`web_search` for nutrition data, `food_lookup` for the user's own foods)
to look up data, retry until the model returns valid TOML, show the
proposed change, and write it only after a three-way confirmation.

The core of this — *route a request through an LLM, validate the structured
output, get human confirmation, return the value* — is a reusable primitive and
lives in a separate library crate, `intake-ai`, so it can be extracted and
reused outside intake. The name deliberately acknowledges that the lib is
currently owned by intake; it will be renamed as part of the eventual split-off.

The system is intentionally provider-neutral: it speaks the OpenAI-compatible
`/v1/chat/completions` API (works with OpenAI, Groq, Mistral, OpenRouter,
Ollama, vLLM, etc.). Web search is a plain function-calling tool executed
in-process — no MCP, no external server packages, no closed ecosystem. Search
uses DuckDuckGo (no API key).

## Workspace structure

```
intake/
├── Cargo.toml              # [workspace] members = ["crates/intake", "crates/intake-ai"]
├── AI-DESIGN.md
└── crates/
    ├── intake-ai/          # the resolve pipeline (temporary name — see Overview)
    │   ├── Cargo.toml      # deps: ureq, serde, serde_json, anyhow (tempfile in tests)
    │   └── src/
    │       ├── lib.rs
    │       ├── settings.rs # AiSettings (serde Deserialize)
    │       ├── search.rs   # web_search tool: DuckDuckGo backend
    │       ├── llm.rs      # LlmBackend trait + ureq impl + agent/tool loop
    │       ├── pipeline.rs # generate_valid<T>: fences → parse → retry
    │       └── confirm.rs  # Confirmer trait only — impls live at the consumer
    └── intake/             # the existing binary, moved as-is
        ├── Cargo.toml      # + intake-ai path dep, similar
        ├── src/            # main, config, display, food, log, edit.rs, confirm.rs (y/n + gated 3-way)
        │   └── ai/         # feature-gated; ai_cmd + prompts/*.md (include_str!)
        └── tests/fixtures/
```

## Crate boundaries

**In `intake-ai`** — the generic primitive. Knows nothing about intake, food,
or TOML:

- `settings.rs` — `AiSettings` (api_key, model, base_url, max_retries,
  max_tool_iterations, timeout_secs, search_timeout_secs, verbose — bool,
  default false; see "Verbose mode"), deriving `Deserialize`
  so consumers map their config onto it directly.
- `search.rs` — the `web_search` tool. Given a query, fetches results from
  DuckDuckGo (no API key; result
  extraction from the HTML result page via `ureq`) and returns plain text
  (title, URL, snippet per result)
  for the model to read. Exposed behind a small `Tool`
  trait so the agent loop is generic.
  Snippet sufficiency for nutrition queries is validated in
  practice — see Open questions for a food-data API refinement.
- `llm.rs` — `LlmBackend` trait; real impl does
  `POST {base_url}/chat/completions` via `ureq` (blocking + rustls); tests use
  a scripted fake backend (no network). Responses are read for
  `reasoning_content` / `reasoning` fields when the provider emits them
  (DeepSeek, OpenRouter, etc.). Agent loop: the caller registers the
  tools it wants available (the lib ships `web_search`); registered tools
  become function definitions; `tool_calls` are executed in-process and
  results fed back; capped at `max_tool_iterations` (default 8).
- `pipeline.rs` — the orchestrator: the `Resolver` struct with `resolve<T>`
  (see "The resolve loop" below), with `generate_valid<T>` as its internal
  parse/retry step: strip markdown
  code fences → caller-supplied `parse: Fn(&str) -> Result<T, String>` closure
  → on failure append the error to the conversation and re-request, up to
  `max_retries` (default 3). Format-agnostic — no `toml` dependency; intake
  supplies the parse closures: plain `toml::from_str` for foods, ops
  deserialize + validation + `apply_ops` for `ai log` (see "`ai log` ops").
- `confirm.rs` — the `Confirmer` trait
  (`Accept` / `Reject` / `Feedback(String)`), the loop's only terminal hook.
  Implementations live at the consumer: intake provides the terminal
  `[y]es` / `[n]o` / `[f]eedback` prompt and the `ConfirmAlways`
  implementation for `--yes`. Proposal *rendering* is a callback supplied by
  the caller.

The lib takes text in and returns the validated `T`; prompt capture
(`$EDITOR` etc.) and confirmation UX are consumer concerns.

**Stays in `crates/intake`** — everything intake-specific:

- clap surface: the reworked tree (bare, `log`, `day`, `summary`, `exercise`,
  `food` group, `completions`) plus the `ai` tree and shared flags; the
  shared `--date` arg (one `Args` definition, flattened into `log` and
  `ai log`); `day` and `summary` own their date args (`[date]` + `-d`)
- prompt capture: `$VISUAL` → `$EDITOR` → `nano` on a temp file with `#`
  guidance comments; unchanged file aborts; clear error when no editor spawns
- a non-gated `[y]es` / `[n]o` confirm helper (+ `--yes`) used by the plain
  `food new`/`food edit` path
- the three-option terminal `Confirmer` (`[y]es` / `[n]o` / `[f]eedback`) and
  `ConfirmAlways` for `--yes`, implementing the lib trait — AI-only,
  `#[cfg(feature = "ai")]` (the one gating exception, see "Feature gating");
  composed on the y/n helper
- the plain `food new`/`food edit` editor + validation + confirm path
  (non-gated — no LLM involvement)
- `[ai]` config wiring (config file → env var → CLI flag resolution)
- per-command default prompt templates — `ai/prompts/*.md` files (schema text
  describing `Food` / `DayLogOps`) embedded via `include_str!`
- proposal rendering via the existing `display::Table` code
- the `DayLogOps` schema with `apply_ops`, the batched `food_lookup` tool,
  and per-command tool registration (see "Tools")
- slug validation + parse-time collision checks (shared by both `food new`
  paths; see "Slug argument")
- the actual writes: food files, `log::write_day`

## The resolve loop (pipeline ownership)

The lib drives the entire loop; intake only supplies inputs. One orchestrator
in `pipeline.rs` owns the conversation and the lifecycle:

```rust
pub struct Resolver<'a> {
    ctx: &'a ResolveContext,             // settings, llm backend, tools
    confirm: &'a dyn Confirmer,          // Accept | Reject | Feedback(String)
    system: String,                      // base prompt — stable across resolve calls
}

impl Resolver<'_> {
    pub fn resolve<T>(
        &self,
        user: &str,                      // the user's request prompt
        parse: impl Fn(&str) -> Result<T, String>,
        present: &dyn Fn(&T) -> String,  // render the proposal (intake's tables)
    ) -> Result<T, ResolveError>          // Exhausted | Rejected | Cancelled | Io
}
```

1. Agent loop (`llm.rs`) until the model returns a final answer with no
   `tool_calls`.
2. Fence-strip + `parse` — on failure, append the error to the conversation
   and re-request, up to `max_retries` (automatic; no human in this loop).
3. On success → `present(&T)` → `confirm()`:
    - `Accept` → return the resolved `T`
    - `Reject` → `ResolveError::Rejected`
    - `Feedback(msg)` → append msg, loop back to step 1 — conversation
      continuity is free because the messages live in this function
4. `ResolveError::Exhausted` carries the last parse error and the model's raw
   output so the caller can surface them.

Consequences:

- Feedback rounds are **uncapped** — the user is the bound; Reject and Ctrl-C
  are the exits. `--yes` short-circuits via `ConfirmAlways`.
- The `Resolver` groups the stable deps (`ctx`, confirmer, system prompt), so a
  consumer constructs it once and reuses it across calls — the per-call
  `resolve` takes only the request-specific inputs. (Matters for the future
  crate split-off; intake's commands each construct one.)
- Intake's entire job: build the system prompt from its per-command template,
  construct a `Resolver` with `ctx`, a confirmer, and the system prompt, supply
  `parse` / `present` (its tables) per call, and write on
  `Ok(T)`. The parse closures are intake-side: plain `toml::from_str` for
  foods; ops deserialize → validate → `apply_ops` for `ai log`.
- The loop is unit-testable as a sequence: scripted fake backend + scripted
  confirmer cover retries, feedback, reject, and exhaustion without network.
- `present` is skipped for `ConfirmAlways` (nothing renders under `--yes`);
  the post-write reprint is the only display in that mode.
- Worst-case cost per resolve round: `max_retries × (max_tool_iterations + 1)`
  ≈ 27 LLM calls with defaults (3 × 9), plus user-driven feedback rounds —
  bounded per round; the user is the overall bound.

## Commands

The surface is one coherent tree: reading is bare / `day` / `summary`,
writing is always `log` / `food` / `exercise`, and AI is a transparent prefix
(`ai`) on the write verbs — every write verb also has a plain, non-AI form:

```
intake                          # today's log
intake log <slug> [servings] [--date D]     # log a food (slug completion)
intake log "<name>" [servings] --calories N --protein N --fiber N --fat N --carbs N --alcohol N
                                # adhoc entry with inline macros
intake day [date] [-d N]        # view a day (days ago)
intake summary [date] [-d N]    # multi-day summary (window)
intake exercise <calories>      # record exercise for today
intake food list                # all foods with per-serving values
intake food show <slug>         # a food's ingredients and per-serving values
intake food new <slug>          # plain: editor + validation + confirm, no AI
intake food edit <slug>         # plain: editor + validation + confirm, no AI
intake completions <shell>      # shell completion script

intake ai log [prompt...] [--date D]   # AI day editing (ops-based)
intake ai food new <slug> [prompt...]  # AI recipe generation
intake ai food edit <slug> [prompt...] # AI recipe editing (slug completion)
```

This reworks the old surface: `add` → `log` (write), the read `log` → `day`,
`show`/`list` → `food show`/`food list`, `adhoc` → `log` with macro flags,
`ai edit-log` → `ai log`, `ai add-recipe` → `ai food new`, `ai edit-recipe` →
`ai food edit`. All existing long-form flags are preserved: `--days-ago`
(short `-d`, on `day`), `--days` (short `-d`, on `summary`), `--install`, the
`--calories` / `--protein` / `--fiber` / `--fat` /
`--carbs` / `--alcohol` flags, and the global `--foods-dir` / `--log-dir`.

`log` disambiguation: any macro flag present selects the adhoc path,
decisively — the name is a free-form title and is never slug-resolved
(`log turkey-chili 2 --calories 500` logs an adhoc entry titled
"turkey-chili" with 500 cal and zeros for the rest). With no macro flags,
the name must resolve to an existing slug → the food path, macros computed
from the file (exactly like today's `cmd_add`); a non-resolving name with no
macros → a clear error pointing at `ai log`. Nothing is ever logged with
silent zero macros.

### Date arguments

- Logging commands (`log`, `ai log`) target a day via `--date D` — long form
  only, no short flag. One shared clap `Args` definition is flattened into
  both commands so they can't drift, with the log-date completion candidates
  attached.
- `--days-ago` is a display convenience and exists only on `day`:
  `[date]` positional + `-d N` / `--days-ago N`. Supplying both `[date]` and
  `-d N` is a usage error (clap `conflicts_with`).
- `summary` keeps its own `[date]` + `-d N` / `--days N` (window) — a
  semantically separate command with its own flags; no sharing.

The AI commands, in full:

| Command | Input | Output | Write |
|---|---|---|---|
| `ai log` | numbered day rows + totals line + 7-day history + user prompt | `DayLogOps` — see "`ai log` ops"; macros for food-derived rows never come from the model | validated and applied by intake, whole-day rewrite via new `log::write_day` |
| `ai food new <slug>` | user prompt + slug positional | `Food` TOML | new `<slug>.toml` in foods dir; an existing slug errors at parse time (see "Slug argument") — never a model retry |
| `ai food edit <slug>` | current food TOML + user prompt | updated `Food` | overwrite food file |

### Plain food editing

`food new <slug>` and `food edit <slug>` never touch the model and are
**not** gated behind the `ai` feature — pure CLI UX, no LLM involvement.
`food new` validates the slug first (strict charset, no existing file — see
"Slug argument"), then opens `$EDITOR` on a schema skeleton (a serialized
canned example with `#` guidance comments, including a `# slug: <slug>`
line); `food edit <slug>` pre-fills the editor with the current
food's TOML. On save: parse and validate
with the same serde structs the AI path uses. A failed save re-opens the
editor with the user's last content preserved and the validation error
prepended as `#` comments — unchanged-file exit aborts; the loop continues
until valid or aborted. On success: render the proposed food via the
existing `display::Table` code, and confirm (`[y]es` / `[n]o`, or `--yes` to
accept). Editor capture and the y/n confirm helper are intake-owned; the AI
path's three-option confirmer and Resolver are simply absent. Food mutation
being longer-winded than logging is fine — it is an uncommon operation.

### Slug argument

`food new` and `ai food new` take the slug as a required CLI positional —
the filename the food will be written to, matching `log` / `show` / `edit`.
The slug is validated strictly (lowercase alphanumerics + dashes only,
matching the existing slug charset; easy to loosen later). An existing slug
errors immediately at parse time: "food 'X' already exists — use
`food edit X` / `ai food edit X` to modify it". The AI returns plain `Food`
TOML — no slug field, the filename is the slug — and intake writes it to
`<slug>.toml`; the title inside may differ from the slug (e.g.
`tjs-lunch.toml` with title "TJs Lunch"). Slugs exist only as food
filenames: log entries store display titles, never slugs.

Shared flags (all `ai` commands, plus `--yes` on plain `food new`/`food edit`):

- `--api-key` — override API key
- `--model` — override model
- `--yes` — skip confirmation, accept
- `--prompt "..."` — provide the prompt inline; conflicts with the
  `[prompt...]` positional; either wins over opening `$EDITOR`
- `--verbose` — print a per-round LLM trace (reasoning, raw output, tool calls,
  parse errors) to stderr

## `ai log` ops

`ai log` never asks the model to reconstruct the day. The model returns
a small op list (`DayLogOps`, intake-owned — `src/ai/ops.rs` inside the gated
module); intake validates it, applies it to the current day log via
`apply_ops`, and proposes the result. Food-derived rows are always computed
by intake, never written by the model:

```toml
[[ops]]
kind = "add-food"            # no macro fields — intake computes them via food.per_serving()
slug = "turkey-chili"
servings = 1.5

[[ops]]
kind = "add-adhoc"           # the only place model-written macros appear
title = "Almonds - 30g"
servings = 1
calories = 164
protein_g = 6.0
fiber_g = 3.5
fat_g = 14.0
carbs_g = 6.0
alcohol_g = 0.0

[[ops]]
kind = "remove"
row = 3                      # index into the numbered list shown in the prompt

[[ops]]
kind = "replace"             # remove + re-add in place; keeps ordering
row = 2
slug = "oatmeal"             # food variant: slug + servings only
servings = 2
# adhoc variant: title + all six macros instead of slug
```

Semantics:

- `row` indices are **1-based** and always refer to the original day log
  exactly as numbered in the prompt; they never shift as ops apply.
  Removals and replacements apply first;
  additions append at the end.
- Validation errors feed the retry loop just like parse failures: unknown
  `slug` (must exist in the foods dir — `food_lookup` results are the model's
  only source of slugs), `row` out of range, duplicate or conflicting ops on
  the same row, `add-adhoc` with missing or invalid macros.
- `exercise_calories` is preserved by construction — `apply_ops` copies it
  through and no op can touch it (an explicit `exercise` op is future work).
- The `ai log` parse closure is: deserialize `DayLogOps` → validate →
  `apply_ops` → new `DayLog`. The lib stays generic;
  `resolve<DayLog>` only sees the final applied day.

### The `food_lookup` tool

The model decides whether an entry is a food row or an adhoc row; intake
never guesses and never converts. `food_lookup` (mechanics: "Tools") is
batched and returns up to five catalog lines per query; matching is
forgiving because a wrong suggestion is harmless — the model chooses, and
the user confirms.

The model's task shrinks to *choosing*: if a result matches the user's
intent, emit `add-food` (slug + servings; macros computed by intake, exact
by construction); if the user wants a modified version or no food exists,
emit `add-adhoc` with its own macros, preserved untouched. No automatic
conversion, no `from food:` marker — intent lives in the op kind, and the
row diff shows the outcome.

The prompt makes lookup mandatory: before emitting any `add-adhoc` op,
include the intended title in a batched `food_lookup` call; a match is a
decision point (use the food, or keep your own macros), not a prohibition
on adhoc entries.

## Context windows

Prompts embed compact snapshots of the user's data, sized per command — full
TOML would bloat the context and bury what matters. Three compact formats,
all showing all six macros regardless of `show_columns` (that config
controls human display, not model context):

- **Entry line** — `title | servings | cal, protein, fiber, fat, carbs, alcohol`,
  the same width-independent format used for the `ai log` row diff. Entry
  lines (including the history window) are the only food-adjacent data
  embedded in prompts.
- **Catalog line** — `slug | title | cal/serv, protein, fiber, fat, carbs, alcohol`,
  per-serving values via `food.per_serving()`. The result format of
  `food_lookup`; never embedded in prompts — the model sees foods only when
  it asks.
- **Totals line** — `totals: cal | protein | fiber | fat | carbs | alcohol`,
  plus `exercise: N` and the configured min/max targets when set. Present in
  `ai log` context only.

Per command:

| Command | Prompt context |
|---|---|
| `ai log` | numbered entry lines for the day being edited, a totals line (which includes `exercise: N`), and the 7 days before the edited day as dated entry lines — hardcoded window, configurability is future work |
| `ai food new <slug>` | none beyond the schema + 2 full sample foods from the user's own foods dir (naming, serving, and ingredient conventions — see "Sample foods") |
| `ai food edit <slug>` | the target food's full TOML + the same 2 sample foods |

Details:

- The history window **anchors to the edited day** (the 7 days before
  `--date`; the edited day itself is the numbered context) and is truncated
  to a cap (first 40 entry lines, oldest dropped) so long streaks cannot
  bloat the prompt. The numbered edit-target rows are never truncated —
  they are the reference the ops point at.
- The totals line lets the model answer prompts like "log dinner to hit my
  protein target".
- A day with no log file is valid context: the numbered list is empty and
  the write creates the file.
- **Sample foods**: `ai food new` and `ai food edit` embed two full `Food`
  TOMLs from the user's own foods dir so new recipes fit the user's
  conventions (titles, serving sizes, ingredient granularity, quantity
  style like "400g" vs "1 tbsp"). Selection is deterministic: first two by
  slug order, preferring foods with ≥2 ingredients; an empty dir falls back
  to the template's canned examples. Catalog lines via `food_lookup` are the
  wrong shape for this — they lack ingredient structure — so the tool stays
  `ai log`-only.

The history window teaches the model the user's logging patterns — title
conventions like "Cherries - 155g", portion sizes, which macros are usually
non-zero — for the adhoc entries it appends mid-edit.

## Tools

The agent loop executes caller-registered tools in-process. Each tool is
exposed to the model as a function definition (name, description,
JSON-schema parameters) on the OpenAI-compatible API, executed on
`tool_calls`, and fed back as a plain string. A failed or timed-out call
returns a short error string to the model and counts against
`max_tool_iterations`.

The `Tool` trait (in `intake-ai`):

```rust
pub trait Tool {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;   // JSON-schema of parameters
    fn execute(&self, params: &serde_json::Value) -> Result<String, String>;
}
```

- **`web_search`** (ships with the lib): a single `query` string; returns
  up to five results as `title | URL | snippet` lines, per-snippet cap
  (~200 chars) and total output cap (~2k chars). DuckDuckGo backend, no
  API key; a fetch failure returns a short error string.
- **`food_lookup`** (intake-side, registered by `ai log` only):
  batched — takes `queries: [string]` (one or more titles) and returns
  per-query matches, so a multi-entry edit costs 1-2 calls instead of one
  per entry. Matching per query:
  1. Normalize both sides: lowercase, strip non-alphanumerics
     (Unicode-aware, e.g. `char::is_alphanumeric`).
  2. Exact matches on slug or title first, then containment (query inside
     slug/title or vice versa). Top ~5 per query, exact matches first,
     deduplicated.
  3. Results render as catalog lines (see "Context windows"), with a total
     output cap. An empty foods dir or no matches returns an explicit empty
     result.

Registration per command: `ai log` → `food_lookup` + `web_search`;
`ai food new <slug>` / `ai food edit <slug>` → `web_search` only — the
slug comes from the CLI (see "Slug argument"), so there is no name
collision for the model to manage; `web_search` covers ingredient
nutrition.

Budget math: batched lookups keep a 10-entry edit at 1-2 tool calls;
`max_tool_iterations` (default 8) covers the typical session with room for
occasional `web_search` calls.

## Shared flow (all three commands)

1. Capture prompt: the `[prompt...]` positional or `--prompt "..."` wins;
   otherwise `$VISUAL` → `$EDITOR` → `nano` opened on a temp file with
   `#` guidance comments; user types prompt and saves. If the file is
   unchanged on exit, abort.
2. Build messages: base prompt (command-specific template, config-overridable)
   → `Resolver` constructor; user prompt → `resolve`.
3. Agent loop: model may call the registered tools (see "Tools");
   results fed back; capped at `max_tool_iterations` (default 8).
4. Strip ```toml code fences, parse into the target struct
   (`Food` / `DayLogOps` — ops are validated and applied, see "`ai log` ops").
5. Retry loop: on parse/validation failure, append the error to the
   conversation and re-request, up to `max_retries` (default 3). On exhaustion,
   print the last error and the model's raw output so the user can fix it
   manually. Errors appended to the conversation follow a fixed shape: quote
   the offending op, state the rule, and give the fix (valid slugs capped at
   ~10, or the valid row range).
6. Show the proposed change: for `ai log` a row-level diff of the applied
   day log vs. the original (one normalized
   line per entry: `title | servings | macros`, additions/removals/changes as
   `-`/`+` pairs via the `similar` crate) followed by the day-log table for
   totals context; for foods the existing `display::Table` rendering
   (`ai food edit` shows before/after food tables). An empty op list is a
   valid proposal: the diff is empty, "no changes" is shown, and the user
   decides (accept → nothing written; feedback → re-run).
7. Three-way confirmation: `[y]es` / `[n]o` / `[f]eedback`. Feedback prompts
   inline for extra instructions and re-runs generation with the conversation
   continued. `--yes` skips proposal rendering and confirmation entirely —
   the write happens and the post-write reprint (step 8) is the only display.
   With `--yes` and an empty op list, print "no changes" and exit.
8. Write the change (or nothing on reject/abort). Before writing, intake
   **reloads the target** (day log or food file); if it differs from the
   snapshot the context was built from, abort with "changed since this
   proposal was generated — re-run" instead of overwriting. For `ai food
   new` the collision check already ran at parse time (see "Slug
   argument"); the write-time recheck aborts if the file appeared
   mid-flow. After a
   successful write, reprint the affected table (day log for `ai log`, food
   for `ai food new`/`ai food edit`).

Steps 2–7 run inside the lib's `resolve` loop (see "The resolve loop"); intake
builds the system prompt, constructs a `Resolver` with `ctx` + its confirmer,
passes the user prompt to `resolve` per call, and performs the write.

## Verbose mode

When `verbose` is set (via `[ai] verbose` or `--verbose`), each LLM round in
the loop prints a short trace to **stderr** — reasoning content when the
provider emits it (e.g. DeepSeek's `reasoning_content`, OpenRouter's
`reasoning`), the model's raw text output before parsing, tool calls
(`[tool] web_search "query"`), and parse errors per retry. Nothing goes to
stdout — proposal tables and the confirm dialog are unaffected. Behavior is
identical with the flag off; the flag only adds visibility.

## Change review & safety

How the confirmation presents each command's proposal:

- `ai log` — a **row diff** built with `similar`: each entry of the applied
  day log is rendered as one width-independent line (`title | servings | macros`),
  and the applied day is diffed against the original, so changes, additions,
  and removals appear as exact `-`/`+` pairs. The day-log table follows for
  totals context. No raw-TOML diffs (unreadable) and no diffs of rendered
  tables (column widths reflow when values change, producing phantom changes
  on unrelated rows).
- `ai food edit` — before/after food tables only; no diff.
- `ai food new` — the new data only; no diff.

`ai log` needs no drop protection: the ops design never reconstructs the
day, so the result is always the original minus explicit `remove` ops plus
additions — entries cannot vanish accidentally. `exercise_calories` is
likewise safe by construction: `apply_ops` copies it through and no op can
change it.

`similar` is an intake-crate dependency: proposal rendering is intake's job
(the lib's `present` callback), so the lib stays generic.

## Configuration

```toml
[ai]
api_key = "..."          # or INTAKE_AI_API_KEY / --api-key
model = "gpt-4o-mini"    # or INTAKE_AI_MODEL / --model
base_url = "..."         # optional; default api.openai.com
max_retries = 3
max_tool_iterations = 8
timeout_secs = 60            # per LLM API call
search_timeout_secs = 15     # per web_search backend fetch
verbose = false          # or --verbose; per-round LLM trace on stderr
log_prompt = "..."       # optional overrides of default templates
food_new_prompt = "..."
food_edit_prompt = "..."
```

- `Config` gets `#[serde(default)] ai: AiConfig` so existing configs and tests
  are unaffected; the `[ai]` table deserializes into `intake_ai::AiSettings`
  (via the wrapper) — no mapping layer. The wrapper field is
  `#[cfg(feature = "ai")]`-gated (see Feature gating).
- Resolution order matches `Config::resolve`: config file → env var → CLI flag.
  The merge happens in `ai_cmd`: it combines the `[ai]` config values, the
   `INTAKE_AI_*` env vars, and the shared `ai` flags into the final
   `AiSettings` before constructing a `Resolver` and calling `resolve`.
- The API key is never logged or printed.
- Timeouts: `timeout_secs` (default 60) bounds each LLM API call;
  `search_timeout_secs` (default 15) bounds each `web_search` fetch.
  Transient transport failures (HTTP 429/5xx) retry up to twice with
  backoff; timeouts and other errors abort with a clear error — no further
  automatic retry (`max_retries` covers only validation failures, not
  transport failures).
  The editor prompt capture and confirmation prompt are user-paced and
  untimed.
- Known limitation: web search requires a model/provider that supports
  function/tool calling. If the endpoint rejects `tools`, error clearly and
  suggest a different model.

## Feature gating

AI support is a Cargo feature on the `intake` binary crate, enabled by
default:

```toml
[features]
default = ["ai"]
ai = ["dep:intake-ai"]
```

- With the default feature, everything in this doc applies.
- Gating is **module-level**, not scattered: `#[cfg(feature = "ai")] mod ai_cmd;`
  in main.rs owns the clap args, the `ai` tree handlers, the prompt
  templates, `INTAKE_AI_*` env resolution, and the Resolver wiring for the
  AI commands — the editor capture and the y/n confirm helper are shared,
  non-gated code (see "Crate boundaries"); no cfg
  attributes inside the module. **One documented exception:** the
  three-option terminal `Confirmer` in `confirm.rs` carries
  `#[cfg(feature = "ai")]` — it implements the lib's `Confirmer` trait and
  exists only for the AI pipeline. The templates are `.md` files under
  `src/ai/prompts/` (`log.md`, `food_new.md`, `food_edit.md`),
  embedded via `include_str!` *inside* the gated module — so they compile into
  the binary only when the `ai` feature is on; no-feature builds embed nothing.
  The plain `food new`/`food edit` editor prefill is a non-gated schema
  skeleton (serialized canned example), not these files.
  main.rs has exactly three cfg sites (module decl, the
  `Ai` subcommand variant, the match arm); config.rs has two (the
  `ai: Option<AiConfig>` field and the gated `AiConfig` wrapper holding
  `intake_ai::AiSettings`).
- Without the feature, the CLI is the reworked surface plus plain `food new`/
  `food edit` (editor + validation + confirm), minus the `ai` tree; the only
  `[ai]`-specific behavior is a one-line stderr
  warning when config.toml contains an `[ai]` table ("config contains `[ai]`
  but this binary was built
  without the `ai` feature; AI commands are unavailable"). Detected by a small
  `#[cfg(not(feature = "ai"))]` block in `Config::load` that parses the raw
  content for the `ai` key — warned-and-ignored, not silently ignored.
- `intake-ai` is a workspace member, not itself gated: its own code and tests
  build and run regardless of intake's feature selection.

## Prompt templates

Templates are `.md` files under `crates/intake/src/ai/prompts/` (`log.md`,
`food_new.md`, `food_edit.md`), embedded at compile time via `include_str!`
(built into the binary). The `[ai]` config keys (`log_prompt`,
`food_new_prompt`, `food_edit_prompt`) override only the **static text** —
the context block is always injected by code on top, so an override cannot
omit the data the model needs. These files are for the model only: the
plain `food new` / `food edit` commands prefill the editor from a non-gated
schema skeleton (a serialized canned example with `#` guidance comments)
instead, since they work without the `ai` feature.

Every template has the same anatomy:

1. **Role and task** — one or two sentences framing the command.
2. **Context block** (code-injected, never part of the `.md`): the
   per-command data from "Context windows".
3. **Schema** — the exact TOML the model must emit (`Food` + `Ingredient`,
   or `DayLogOps`), mirroring `AGENTS.md` / the serde structs.
4. **Rules** — all macro fields required, no default of zero; if unsure
   about any macro, call `web_search` to look up nutrition data (prefer
   per-100g values, e.g. USDA, and scale by serving size); output TOML
   only, no prose, no surrounding fenced blocks.
5. **Examples** — 1-2 short canned examples illustrating the schema and
   conventions. For `ai log` the history window doubles as live examples of
   the user's actual patterns; for `food_new`/`food_edit` the context
   embeds two of the user's own foods (see "Context windows"), and the
   canned examples serve as the fallback when the foods dir is empty.
6. **Output format** — TOML only; fences are stripped anyway.

Per-command content:

- `log.md` (`ai log`): the ops schema (see "`ai log` ops"); the numbered
  context rows are references only — never re-emit them; before any
  `add-adhoc` op, include the title in a batched `food_lookup` call and
  decide per result: `add-food` when it matches the user's intent (macros
  computed by intake), `add-adhoc` with your own macros (all six) for
  one-offs and modified versions of foods; `row` indices are 1-based and
  refer to the numbered list exactly as shown — they never shift.
- `food_new.md` (`ai food new <slug>`): the `Food` schema; the slug is
  supplied on the command line and the title may differ from it (see "Slug
  argument"); `notes` is optional.
- `food_edit.md` (`ai food edit <slug>`): the target food's TOML is the
  context; preserve all untouched fields; the 2 sample foods for
  consistency; before/after is shown at confirm.

## Implementation steps

1. **Workspace restructure** — root `Cargo.toml` → `[workspace]` with
   `resolver = "2"`, `members = ["crates/intake", "crates/intake-ai"]`; move
   the existing package to `crates/intake/` (src, tests/fixtures, Cargo.toml)
   with no code changes. Verify all four quality gates stay green at the root.
2. **Non-AI CLI rework** — the surface rework (`log` write + `day` read +
   bare `intake`, `adhoc` merged into `log` flags, shared `--date`,
   `day`'s `-d`); the `food` group with `list`/`show`; plain `food new <slug>`/
   `food edit <slug>` (non-gated: slug validation + parse-time collision
   check, editor capture, schema-skeleton prefill,
   validation, y/n confirm helper, confirm); `log::write_day`.
   No `intake-ai` dependency; all four gates green.
3. **`crates/intake-ai`** — settings, search, llm, pipeline, confirm (trait
   only) modules with unit tests. Text in, validated `T` out; no editor, no
   terminal confirmer impl.
4. **AI integration** — `[features] default = ["ai"]`,
   `ai = ["dep:intake-ai"]`; `[ai]` config wiring + no-feature warning, the
   feature-gated `ai` tree (`ai log`, `ai food new`, `ai food edit`) with the
   Resolver + confirmation flow, the three-option terminal `Confirmer`
   (+ `ConfirmAlways`, `#[cfg(feature = "ai")]`), per-command prompt
   template files (`src/ai/prompts/*.md`), the `Tool` trait wiring
   (`web_search` +
   batched `food_lookup` with per-command registration), the `DayLogOps`
   schema with `apply_ops`, and the stale-write check.
5. **Docs** — AGENTS.md / README `[ai]` section and path touch-ups.

## Testing

- `intake-ai` unit: fence stripping; retry loop via scripted fake backend
  (bad → good TOML in sequence); search result parsing from canned HTML;
  tool-call execution roundtrip; confirmation decision mapping (a scripted
  `Confirmer` impl driving the loop); timeout aborts with a clear error
  (hang-mode fake backend + tiny timeout, no slow tests); transport retry
  (scripted 429/5xx then success, no slow tests); verbose mode —
  scripted fake backend responses carrying `reasoning_content` produce the
  expected stderr trace lines.
- `intake` unit: prompt capture via a mocked `$EDITOR` (spawns, unchanged
  file aborts, no `$EDITOR` set errors); the y/n confirm helper (always
  built); the three-option confirmer decision mapping (in the `ai` build);
  plain `food new`/`food edit` save-failure loop (error comments + last
  content preserved, unchanged aborts); `--yes` skips proposal rendering;
  `[ai]` config parsing and env/flag resolution;
  `log::write_day` roundtrip; clap parsing of the reworked surface — bare
  `intake` shows today, `log` write vs. `day` read, `log` food-vs-adhoc
  disambiguation (macro flags always win — a slug-matching name plus a macro
  flag logs an adhoc entry; a non-slug name with no macros errors), the
  shared `--date` arg on `log` and `ai log` (no short
  form), `day` `[date]`/`-d` conflict errors, `summary` `-d` window, the
  `food` group with plain `food new`/`food edit`, the
  `ai` tree and shared flags, `--prompt` vs. `[prompt...]` conflict;
  `apply_ops` (add-food / add-adhoc / remove / replace; row out of
  range; duplicate and conflicting ops; additions append, replacements keep
  position); `food_lookup` matching (normalized exact match on slug and
  title; containment fallback; top-N ordering; batch mode — multiple
  queries in one call; empty result for unknown foods and for an empty
  foods dir); context assembly (totals line, history window anchored to the
  edited day, truncation cap, empty day, sample foods embedded for
  `ai food new`/`edit` with empty-dir canned fallback); stale-write abort (target changed
  between context build and write); empty ops proposal (empty diff, no
  write); slug argument (strict charset validation; parse-time collision
  error on both `food new` and `ai food new`, suggesting `food edit`);
  write-time recheck for `ai food new`;
  row diff (one macro
  changed → exactly one `-`/`+` pair; entry added → one `+` line; unchanged →
  empty diff); stale-`[ai]`-config warning detection (feature off);
  prompt-file drift guard (each prompt `.md` under `src/ai/prompts/` contains
  its key schema field names, e.g. `protein_g` / `add-adhoc` — fails if serde
  structs change without prompt updates).
- Quality gates (workspace root): `cargo test`, `cargo clippy -- -D warnings`,
  `cargo fmt --check`, `cargo build`, plus `cargo test --no-default-features`
  and `cargo build --no-default-features` for the no-AI configuration.

## Docs

Update `AGENTS.md` and `README.md` with the `[ai]` config section, a note on
the DuckDuckGo-backed `web_search` tool, and corrected paths for the workspace
layout.

## Open questions / future work

- Keyed search backends (Brave, Tavily) and self-hosted SearXNG.
- A caller-registered `nutrition_search` tool backed by the USDA FoodData
  Central API (free key; structured JSON with per-100g nutrients) — higher
  fidelity than web-search snippets for the nutrition use case. v1 uses
  simple DuckDuckGo `web_search` only; revisit if snippets prove
  insufficient. Distinct from the local `food_lookup` tool (which searches
  the user's own foods dir).
- A specialized macro agent exposed to the main agent as a tool
  (`estimate_macros`): a nested `resolve` (one new template file + a `Tool`
  impl running a sub-loop) that takes a food description + portion and
  returns exact per-serving macros — fixes the arithmetic-scaling and
  macro-completeness error classes and isolates the main agent's tool
  budget. Details not scoped: caching, output contract/confidence, nested
  budget accounting, and whether it rides on `web_search` from day one or
  lands with `nutrition_search`.
- Templating the template: variables/placeholders inside the prompt `.md`
  files (context blocks are currently injected by code only, so overrides
  cannot reach them).
- Streaming token output from the model (independent of verbose mode, which
  works without streaming).
- If `ai log` proves slower or less reliable than expected for simple appends,
  an add-only mode (no remove/replace ops, no day context or diff) is a thin
  variant.
- Making the `ai log` history window configurable (`[ai] history_days`); v1
  hardcodes 7 days.
- A `log_lookup` tool as the sibling of `food_lookup` (given a date or range,
  return the day logs as entry lines) — would later expand or replace the
  hardcoded history window, letting the model pull older logs on demand.
- Optionally, search tools for each domain on top of lookup: a `food_search`
  (fuzzy/full-text search across food titles, ingredients, and notes) and a
  `log_search` (search past log entries by title or macros, e.g. "when did I
  last log oatmeal") — useful once catalogs and histories outgrow what a
  lookup-by-name or fixed window can cover. `food_search` would serve the
  recipe builders (`ai food new`/`edit`) once their catalogs outgrow the
  two embedded sample foods.
- An optional `exercise_calories` op in `ai log` (v1: preserved structurally
  by `apply_ops`, unchangeable).
- Renaming `intake-ai` (e.g. `resolve-ai`, `llm-resolve`) and publishing it to
  crates.io as part of the split-off.
