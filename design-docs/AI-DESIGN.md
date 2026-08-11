# Design Doc: AI Commands for intake

Status: Implemented

## Overview

Add AI-powered commands to the `intake` CLI: generate foods, and edit
existing log days and foods. All commands share one
pipeline: capture a user prompt (via a `[prompt...]` positional, `--prompt`,
or `$EDITOR`), send it to an LLM
along with a base prompt, let the model use function-calling tools
(`usda_search` for nutrition data, `food_lookup` for the user's
own foods)
to look up data, retry until the model returns valid TOML, show the
proposed change, and write it only after a three-way confirmation.

The core of this — *route a request through an LLM, validate the structured
output, get human confirmation, return the value* — is a reusable primitive and
lives in a separate library crate, `intake-ai`, so it can be extracted and
reused outside intake. The name fits: the crate takes in data from an AI.

The system is intentionally provider-neutral: it speaks the OpenAI-compatible
`/v1/chat/completions` API (works with OpenAI, Groq, Mistral, OpenRouter,
Ollama, vLLM, etc.). Nutrition lookup is a plain function-calling tool
executed in-process — no MCP, no external server packages, no closed
ecosystem. v1 nutrition data comes from the USDA FoodData Central API (free
API key, structured per-100g JSON, shipped as `usda_search`); a
generic web search and page fetch is future work (see Open questions).

## Workspace structure

`intake-ai` is a new workspace member under `crates/`; the existing
`intake` package stays at the workspace root. Only the new lib lives
under `crates/`.

## Crate boundaries

**In `intake-ai`** — the generic primitive. Knows nothing about intake, food,
or TOML:

- `settings.rs` — `Settings` (model, base_url — both required; api_key
  optional, for endpoints without auth; max_retries, max_tool_calls,
  timeout_secs, trace_requests, trace_responses — both bool, default false;
  see "Tracing"), a plain data type (no `Deserialize`) with no `Default`:
  the endpoint fields are mandatory constructor args to
  `Settings::new(base_url, model, api_key)`, which fills the operational
  defaults (`DEFAULT_MAX_RETRIES` 3, `DEFAULT_MAX_TOOL_CALLS` 20,
  `DEFAULT_TIMEOUT_SECS` 60, tracing off). The library stays
  format-agnostic: it only takes the struct, and consumers decide how to
  populate it and which provider to target.
- `usda.rs` — the USDA FoodData Central-backed nutrition tool:
  `usda_search` (batched queries → candidate foods with per-100g macros).
  Hits `api.nal.usda.gov/fdc/v1` via `ureq` (blocking + rustls); no HTML
  scraping — the API returns structured JSON. Exposed behind a small
  `Tool` trait so the agent loop is generic.
- `llm.rs` — `LlmBackend` trait; real impl does
  `POST {base_url}/chat/completions` via `ureq` (blocking + rustls); tests use
  a scripted fake backend (no network). Responses are read for
  `reasoning_content` / `reasoning` fields when the provider emits them
  (DeepSeek, OpenRouter, etc.). Agent loop: the caller registers the
  tools it wants available (the lib ships the `usda_search`
  tool); registered tools
  become function definitions; a response is final iff `tool_calls` is
  absent — while it is non-empty, all `tool_calls` in the response are
  executed in-process (each execution, successful or failed, counts against
  `max_tool_calls`, default 20) and the results fed back, one message per
  call id. When the budget is exhausted, no further tool executes: a single
  "tool call budget exhausted (max N) — produce your final answer with the
  data you have" message is appended, the *next* model response is taken
  unconditionally as the loop's last (even if it still names tools), and the
  parse step runs; retries after that re-request without tools.
- `pipeline.rs` — the orchestrator: the `Resolver` struct with `resolve<T>`
  (see "The resolve loop" below); the parse/retry step — strip markdown
  code fences → caller-supplied `parse: Fn(&str) -> Result<T, String>` closure
  → on failure append the error to the conversation and re-request, up to
  `max_retries` (default 3) — is inlined in `resolve`. Format-agnostic — no
  `toml` dependency; intake
  supplies the parse closures: plain `toml::from_str` for foods, ops
  deserialize + validation + `apply_ops` for `ai log` (see "`ai log` ops").
- `confirm.rs` — the `Confirmer` trait
  (`Accept` / `Reject` / `Feedback(String)`), the loop's only terminal hook.
  Implementations live at the consumer: intake provides the terminal
  `[y]es` / `[n]o` / `[f]eedback` prompt. `--yes` auto-accept lives in the
  pipeline itself (`ResolveContext::auto_accept`), skipping the render and
  the confirmer. Proposal *rendering* is a callback supplied by the caller.

The lib takes text in and returns the validated `T`; prompt capture
(`$EDITOR` etc.) and confirmation UX are consumer concerns.

**Stays in `intake`** (the root package) — everything intake-specific:

- clap surface: the existing command tree plus the `ai` tree and shared
  flags; the shared `--date` arg (one `Args` definition, flattened into
  `log` and `ai log`); `day` and `summary` own their date args (`[date]` +
  `-d`)
- prompt capture: `$VISUAL` → `$EDITOR` → `nano` on a temp file with `#`
  guidance comments; unchanged file aborts; clear error when no editor spawns
- a non-gated `[y]es` / `[n]o` confirm helper (+ `--yes`) used by the plain
  `food new`/`food edit` path
- the three-option terminal `Confirmer` (`[y]es` / `[n]o` / `[f]eedback`)
  implementing the lib trait — AI-only, living in the gated `src/ai/confirm.rs`
  (no cfg of its own). It reuses the same
  `y`/`yes`/`n`/`no` vocabulary as the y/n helper above, deliberately
  duplicated in its own tri-state classifier rather than composed: the
  vocabularies are tiny and stable, and the AI classifier's `f`/`feedback`
  branch is inherently AI-only, so a fall-through layering would couple the
  gated module to the shared file for no real gain
- the plain `food new`/`food edit` editor + validation + confirm path
  (non-gated — no LLM involvement)
- `[ai]` config wiring (config file → env var → CLI flag resolution)
- per-command default prompt templates — `ai/prompts/*.md` files (schema text
  describing `Food` / `DayLogOps`) embedded via `include_str!`
- proposal rendering via the existing `display::Table` code
- the `DayLogOps` schema with `apply_ops`, the batched `food_lookup` tool,
  and per-command tool registration (see "Tools")
- name validation + parse-time collision checks (shared by both `food new`
  paths; see "Name argument")
- the actual writes: food files, and the checked day write
  (`src/ai/write.rs::write_day_checked` — see "Change review & safety")

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
    ) -> Result<T, ResolveError>          // Exhausted | Rejected | Internal
}
```

1. Agent loop (`llm.rs`) until the model returns a final answer with no
   `tool_calls` — or the tool-call budget is exhausted: then one
   budget-exhausted message is sent, the next response is taken
   unconditionally as the loop's last, and parsing proceeds (see `llm.rs`).
2. Fence-strip + `parse` — on failure, append the error to the conversation
   and re-request, up to `max_retries` (automatic; no human in this loop).
   After tool-budget exhaustion these re-requests carry no tools — the
   parse-error message notes that and asks for a final answer from the data
   already gathered.
3. On success → `present(&T)` → `confirm()`:
    - `Accept` → return the resolved `T`
    - `Reject` → `ResolveError::Rejected`
    - `Feedback(msg)` → append msg, loop back to step 1 — conversation
      continuity is free because the messages live in this function
4. `ResolveError::Exhausted` carries the last parse error and the model's raw
   output so the caller can surface them.

Consequences:

- Feedback rounds are **uncapped** — the user is the bound; Reject and Ctrl-C
  are the exits. `--yes` short-circuits via `ResolveContext::auto_accept`,
  returning the first resolved value without rendering or confirming.
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
- `present` is skipped under `auto_accept` (nothing renders under `--yes`);
  the post-write reprint is the only display in that mode.
- Worst-case cost per resolve round: `max_retries × (max_tool_calls + 2)`
  ≈ 66 LLM calls with defaults (3 × 22) — the +2 is the budget-exhausted
  round and the final answer — plus user-driven feedback rounds —
  bounded per round; the user is the overall bound. Each feedback round
  restarts the full per-round budget, so the worst case is
  `≈66 × (1 + feedback rounds)` calls (e.g. ~200 for two feedback rounds).
  This counts calls, not tokens: each call re-sends the whole conversation,
  so token volume grows with the number of rounds (worst case ~O(rounds²),
  in practice low tens of thousands of tokens per attempt, since per-round
  growth is bounded by the tool output caps and the fixed retry-error
  shape).

## Commands

The AI surface is a transparent prefix on the two write verbs it serves —
`log` and `food new` / `food edit` — both of which keep their plain,
non-AI forms:

```
intake ai log [prompt...] [--date D]   # AI day editing (ops-based)
intake ai food new <name> [prompt...]  # AI recipe generation
intake ai food edit <name> [prompt...] # AI recipe editing (name completion)
```

`ai` with no subcommand is a usage error: clap requires a subcommand, so
bare `intake ai` exits 2 with the `ai log` / `ai food new` / `ai food edit`
usage listed. There is deliberately no `ai rm` or `ai exercise`: food
removal gains nothing over the plain `food rm`, and log removal is already
an `ai log` op (see "`ai log` ops").

`log` disambiguation: any macro flag present selects the adhoc path,
decisively — the name is a free-form title and is never name-resolved
(`log turkey-chili 2 --calories 500` logs an adhoc entry titled
"turkey-chili" with 500 cal and zeros for the rest). With no macro flags,
the name must resolve to an existing food name → the food path, macros computed
from the file; a non-resolving name with no
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
| `ai log` | numbered day rows + totals line + history digest + user prompt | `DayLogOps` — see "`ai log` ops"; macros for food-derived rows never come from the model | validated and applied by intake, whole-day rewrite via checked `write_day_checked` (`src/ai/write.rs`) |
| `ai food new <name>` | user prompt + name positional | `Food` TOML | new `<name>.toml` in foods dir; an existing name errors at parse time (see "Name argument") — never a model retry |
| `ai food edit <name>` | current food TOML + user prompt | updated `Food` | overwrite food file |

### Name argument

`food new` and `ai food new` take the name as a required CLI positional —
the filename the food will be written to, matching `log` / `show` / `edit`.
The name is validated exactly like every other food-name input in intake:
the existing `FoodName` parse (`FromStr` in `food.rs`), which accepts any
single normal filename component — uppercase, spaces, and so on work — and
rejects everything else (empty, path separators, `.` / `..`). An existing name
errors immediately at parse time: "food 'X' already exists — use
`food edit X` / `ai food edit X` to modify it". The AI returns plain `Food`
TOML — no name field, the filename is the name — and intake writes it to
`<name>.toml`; the title inside may differ from the name (e.g.
`tjs-lunch.toml` with title "TJs Lunch"). Names exist only as food
filenames: log entries store display titles, never names.

Shared flags (all `ai` commands, plus `--yes` on plain `food new`/`food edit`):

- `--api-key` — override API key
- `--model` — override model
- `--yes` — skip confirmation, accept
- `--prompt "..."` — provide the prompt inline; conflicts with the
  `[prompt...]` positional; either wins over opening `$EDITOR`
- `--trace-requests` — print the request messages sent to the model to
  stderr (see "Tracing")
- `--trace-responses` — print the model's responses (reasoning, raw output,
  tool calls, parse errors) to stderr (see "Tracing")

## `ai log` ops

`ai log` never asks the model to reconstruct the day. The model returns
a small op list (`DayLogOps`, intake-owned — `src/ai/ops.rs` inside the gated
module); intake validates it, applies it to the current day log via
`apply_ops`, and proposes the result. Food-derived rows are always computed
by intake, never written by the model:

```toml
[[ops]]
kind = "add-food"            # no macro fields — intake computes them via food.per_serving()
name = "turkey-chili"
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
name = "oatmeal"             # food variant: name + servings only
servings = 2
# adhoc variant: title + all six macros instead of name
```

Semantics:

- `row` indices are **1-based** and always refer to the original day log
  exactly as numbered in the prompt; they never shift as ops apply. The
  result is built from the original list: `remove` drops its target row;
  `replace` rewrites its target row's content, keeping the row's original
  position even if other rows were removed; `add-*` ops append at the end.
  Ops on the same row conflict (validation error), so application order
  between removes and replaces never matters. Entries cannot be inserted
  mid-list or reordered; the row diff shows the resulting order, and
  reordering is deferred.
- Validation errors feed the retry loop just like parse failures: unknown
  `name` (must exist in the foods dir — `food_lookup` results are the model's
  only source of names), `row` out of range, duplicate or conflicting ops on
  the same row, `add-adhoc` with missing or invalid macros.
- `add-adhoc` macro fields are the same exact-decimal types as food files
  (`Calories`/`Grams`): whole and fractional values are both valid, and
  the model must not round fractional macros to whole numbers.
- Arithmetic is checked like everywhere else in intake: per-serving
  products, entry totals, and the applied day's totals all use checked
  math. An overflow — e.g. an `add-food` whose `servings` makes a macro
  product exceed the decimal range — is a validation error like any
  other: the parse closure returns `Err(String)`, the retry loop
  re-requests with a fix hint ("smaller `servings`, or an `add-adhoc` op
  with explicit macros"), and retries exhausted → `Exhausted` with the
  raw output. Overflow never panics and never leaks out of the closure
  as an `Internal` error — it is the model's fault, so it must be fixable
  through the retry loop.
- An applied day with no entries and no `exercise_calories` deletes the day
  file instead of writing one — the same convention as `remove_entry`, with
  the same directory sync — so `day` reports "No entries" rather than an
  empty table. The row diff already shows every row removed, so the proposal
  still states what will happen.
- `exercise_calories` is preserved by construction — `apply_ops` copies it
  through and no op can touch it (an explicit `exercise` op is future work).
- The `ai log` parse closure is: deserialize `DayLogOps` → validate →
  `apply_ops` → new `DayLog`, each step returning `Err(String)` for the
  retry loop. The lib stays generic;
  `resolve<DayLog>` only sees the final applied day.

### The `food_lookup` tool

The model decides whether an entry is a food row or an adhoc row; intake
never guesses and never converts. `food_lookup` (mechanics: "Tools") is
batched and returns up to five catalog lines per query; matching is
forgiving because a wrong suggestion is harmless — the model chooses, and
the user confirms.

The model's task shrinks to *choosing*: if a result matches the user's
intent, emit `add-food` (name + servings; macros computed by intake, exact
by construction); if the user wants a modified version or no food exists,
emit `add-adhoc` with its own macros, preserved untouched — sourced from the
user's own past logs (the history digest, scaled to the portion) when one
exists, from a `usda_search` per-100g result scaled to the portion when it
doesn't, and estimated only when
neither applies (restaurant meals, non-US items). No automatic
conversion, no `from food:` marker — intent lives in the op kind, and the
row diff shows the outcome.

The prompt makes lookup mandatory only when local data is absent: an item
already in the history digest is reused directly (see "Context windows"),
and any other intended `add-adhoc` title must appear in a batched
`food_lookup` call before going online; a match is a decision point (use the
food, or keep your own macros), not a prohibition on adhoc entries. USDA
round-trips are the last resort, so reuse-heavy edits typically cost zero
tool calls.

## Context windows

Prompts embed compact snapshots of the user's data, sized per command — full
TOML would bloat the context and bury what matters. Three compact formats,
all showing all six macros regardless of `show_columns` (that config
controls human display, not model context):

- **Entry line** — `title | servings | cal, protein, fiber, fat, carbs, alcohol`,
  the same width-independent format used for the `ai log` row diff. Entry
  lines (including the history window) are the only food-adjacent data
  embedded in prompts. The history digest appends an occurrence count:
  `… ×N`.
- **Catalog line** — `name | title | cal/serv, protein, fiber, fat, carbs, alcohol`,
  per-serving values via `food.per_serving()`. The result format of
  `food_lookup`; never embedded in prompts — the model sees foods only when
  it asks.
- **Totals line** — `totals: cal | protein | fiber | fat | carbs | alcohol`,
  plus `exercise: N` when the day has exercise calories, and the configured
  min/max targets when set. Present in `ai log` context only.

Per command:

| Command | Prompt context |
|---|---|
| `ai log` | numbered entry lines for the day being edited, a totals line (which includes `exercise: N` when exercise was logged), and a history digest of the `history_days` days before the edited day — distinct entry lines with occurrence counts, count-sorted |
| `ai food new <name>` | none beyond the schema + 3 full sample foods from the user's own foods dir (naming, serving, and ingredient conventions — see "Sample foods") |
| `ai food edit <name>` | the target food's full TOML + the same 3 sample foods |

Details:

- The history window **anchors to the edited day** (`history_days` days
  before `--date`, default 14, set via `[ai] history_days`; the edited day
  itself is the numbered context). The raw window is capped before dedup
  (first 200 entry lines, oldest dropped) so long streaks cannot bloat the
  prompt, then collapsed into distinct entry lines with `×N` occurrence
  counts, sorted by count (ties: most recent first). An empty window omits
  the digest section. The numbered edit-target rows are never truncated —
  they are the reference the ops point at.
- The totals line lets the model answer prompts like "log dinner to hit my
  protein target".
- A day with no log file is valid context: the numbered list is empty and
  the write creates the file.
- **Sample foods**: `ai food new` and `ai food edit` embed three full `Food`
  TOMLs from the user's own foods dir so new recipes fit the user's
  conventions (titles, serving sizes, ingredient granularity, quantity
  style like "400g" vs "1 tbsp"). Selection is deterministic and
  diversity-first, so the samples cover the catalog's range rather than its
  first entries: prefer one complex recipe (≥3 ingredients), one food with
  `notes`, and one simple food — each slot filled in name order, with
  fallback across slots when one is empty (a smaller catalog yields fewer
  samples, never duplicates). An empty dir falls back to the template's
  canned examples. Catalog lines via `food_lookup` are the
  wrong shape for this — they lack ingredient structure — so the tool stays
  `ai log`-only.

The history digest teaches the model the user's logging patterns — title
conventions like "Cherries - 155g", portion sizes, which macros are usually
non-zero — for the adhoc entries it appends mid-edit. Because re-used
entries are explicit (counts), reuse-heavy edits can be answered from the
digest alone, skipping `food_lookup` and USDA round-trips entirely. The
digest is the **first** source for macros: an entry matching the user's
request is copied and scaled to the requested portion by the model —
preferable to tool round-trips and to estimation, since it reflects the
user's own logging. This model-side scaling is a deliberate exception
to the "the model never does scaling arithmetic" rule: digest-sourced
macros are accepted as estimate-grade (like any USDA-sourced
`add-adhoc` — `usda_search` returns per-100g values, scaled to the portion
by the model, see "Tools"), and the proposal diff shows every value
before confirmation.

## Tools

The agent loop executes caller-registered tools in-process. Each tool is
exposed to the model as a function definition (name, description,
JSON-schema parameters) on the OpenAI-compatible API, executed on
`tool_calls`, and fed back as a plain string. A failed or timed-out call
returns a short error string to the model and counts against
`max_tool_calls` like a successful one.

The `Tool` trait (in `intake-ai`):

```rust
pub trait Tool {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;   // JSON-schema of parameters
    fn execute(&self, params: &serde_json::Value) -> Result<String, String>;
}
```

- **`usda_search`** (ships with the lib): batched — `queries: [string]`;
  per query, hits USDA `/foods/search` and returns up to five candidates as
  `FDC id | name | portion size (when FDC reports one — branded foods) |
  per-100g macros` lines (all six macros;
  the name carries the variant — "Rice, white, cooked" vs raw — so
  raw/cooked ambiguity is explicit, not buried in a snippet). Per-query
  and total output caps (~2k chars). A fetch failure or rate limit (429)
  returns a short error string, counting against the budget like any
  failure. The model scales the per-100g values to the requested portion
  itself (portion ÷ 100 × each value, rounded to whole calories and 0.1 g);
  the earlier `usda_get` companion (code-side exact scaling) was dropped in
  v2 — it hit the `/food/{fdcId}` endpoint whose nutrient payload nests the
  id under `nutrient.id` and the value under `amount` (unlike search's flat
  `nutrientId`/`value`), so its parsing always read all-zero macros, and the
  search endpoint already carries every value the model needs.
- **`food_lookup`** (intake-side, registered by `ai log` only):
  batched — takes `queries: [string]` (one or more titles) and returns
  per-query matches, so a multi-entry edit costs 1-2 calls instead of one
  per entry. Matching per query:
  1. Normalize both sides: lowercase, fold diacritics (`é`→`e`), strip
     non-alphanumerics (Unicode-aware, e.g. `char::is_alphanumeric`).
  2. Rank by a tiered all-integer score: exact match on name or title,
     then token overlap (query words equal to or prefixing name/title
     words — word boundaries survive normalization, so reordering and
     extra words still hit), then character-bigram Dice similarity
     (catches typos and partial words). Ratios compare by integer
     cross-multiplication, never floats; ties break alphabetically.
     Zero-score candidates drop, so unknown foods still return an
     explicit empty result. Top ~5 per query, deduplicated.
  3. Results render as catalog lines (see "Context windows"), with a total
     output cap. An empty foods dir or no matches returns an explicit empty
     result.

Registration per command: `ai log` → `food_lookup` + `usda_search`;
`ai food new <name>` / `ai food edit <name>` → `usda_search` — the name
comes from the CLI (see "Name argument"), so there
is no naming collision for the model to manage; the USDA tool covers
ingredient nutrition.

Why USDA-only v1: a general web search's ~200-char snippets cannot carry
all six macros plus serving context, and extraction + scaling would land
on the model (see Open questions for the deferred generic-search design).
The USDA API returns structured per-100g JSON with explicit variant names,
so the model only picks the variant; the per-100g→portion scaling it
performs is estimate-grade, accepted like digest-sourced scaling and shown
in the proposal diff. Cost: a free API key
(fdc.nal.usda.gov) and gaps for restaurant meals, branded items absent
from FDC, and non-US foods — the `web_search` + `fetch_url` follow-up
covers those.

Budget math: batching keeps a 10-entry edit at 1-2 tool calls (one
`usda_search` batch, occasional re-search) — and the
history digest lets reuse-heavy edits land at zero tool calls;
`max_tool_calls` (default 20) is a loose ceiling rather than a tight fit —
revisit when the `web_search` + `fetch_url` follow-up lands (see Open
questions).
Exhaustion is not an error to the user: the loop sends one
budget-exhausted message, takes the model's next response unconditionally,
and proceeds to the parse step (see `llm.rs`).

## Shared flow (all three commands)

1. Capture prompt: the `[prompt...]` positional or `--prompt "..."` wins;
   otherwise `$VISUAL` → `$EDITOR` → `nano` opened on a temp file with
   `#` guidance comments; user types prompt and saves. If the file is
   unchanged on exit, abort.
2. Build messages: base prompt (command-specific template, config-overridable)
   → `Resolver` constructor; user prompt → `resolve`.
3. Agent loop: model may call the registered tools (see "Tools");
   results fed back; at most `max_tool_calls` (default 20) executions, then
   one budget-exhausted message and the next response is taken
   unconditionally.
4. Strip ```toml code fences, parse into the target struct
   (`Food` / `DayLogOps` — ops are validated and applied, see "`ai log` ops").
5. Retry loop: on parse/validation failure, append the error to the
   conversation and re-request, up to `max_retries` (default 3) — without
   tools after budget exhaustion. On exhaustion,
   print the last error and the model's raw output so the user can fix it
   manually. Errors appended to the conversation follow a fixed shape: quote
   the offending op, state the rule, and give the fix (valid names capped at
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
8. Write the change (or nothing on reject/abort — both exit 0; see "Exit
   codes"). Before writing, intake
   **reloads the target** (day log or food file); if it differs from the
   snapshot the context was built from, abort with "changed since this
   proposal was generated — re-run" instead of overwriting. For `ai log` the
   reload, the comparison, and the write run inside the same log-directory
   lock (`lock_log_dir`) as the existing read-modify-write paths
   (`append_entry`, `set_exercise_calories`, `remove_entry`), so the recheck
   cannot race a concurrent `log` / `rm` between check and write — mirroring
   `remove_entry`'s expected-entry equality. Foods have no directory lock
   (unlike logs), so the `ai food edit` reload-compare-write is not atomic
   against a concurrent `food edit` landing between the check and the
   write; the window is small and the failure mode is a clobbered concurrent
   edit, which v1 accepts — the check still catches the common case of a
   change made while the proposal was being generated. For `ai food
   new` the collision check already ran at parse time (see "Name
   argument"); the write-time recheck aborts if the file appeared
   mid-flow. After a
   successful write: if the proposal was presented (interactive
   confirmation), print a one-line confirmation — `Logged to {date}` for
   `ai log`, `Wrote {path}` for `ai food new`/`ai food edit` — since the
   proposal already showed the full table. When the proposal was not
   presented (`--yes`), reprint the affected table (day log for `ai log`,
   food for `ai food new`/`ai food edit`) as the only display.

Steps 2–7 run inside the lib's `resolve` loop (see "The resolve loop"); intake
builds the system prompt, constructs a `Resolver` with `ctx` + its confirmer,
passes the user prompt to `resolve` per call, and performs the write.

Non-TTY invocation: confirmation reads stdin, so piped answers work —
`echo y | intake ai log` accepts "y" (POSIX `rm -i` behavior). Closed
stdin is a decline: the confirmer prints a stderr hint and returns
`Reject`, so it exits 0 like any other non-affirmative answer (see "Exit
codes"); the `[f]eedback` sub-prompt follows the same rule. Prompt capture
never reads stdin — without `[prompt...]`/`--prompt` and without a usable
editor it errors, it does not block.

## Tracing

Tracing is off by default and split into two independent toggles, each
available as a config key (`[ai] trace_requests` / `[ai] trace_responses`)
and a shared CLI flag (`--trace-requests` / `--trace-responses`); config
and flags OR together. The lib emits **structured events** — the agent loop
and resolve loop report `MessagesSent`, `Response`, and `ParseError`
events to a consumer-supplied observer (`TraceObserver`); intake's
`src/ai/trace.rs` renders them, so all presentation lives outside the lib:

- **`trace_requests`** — print the request messages sent to the model to
  **stderr**, each message exactly once, on the round it first enters the
  conversation: role-prefixed lines (`[system]`, `[user]`, `[assistant]`,
  `[tool:{call id}]`), bracketed per round by `--- to model ---` /
  `--- end to model ---`. Later rounds print only what is new — tool
  results, parse-error retries, feedback — so stderr reads as a
  conversation transcript rather than a repeated dump.
- **`trace_responses`** — print each response as it arrives as one block,
  bracketed by `--- from model ---` / `--- end from model ---`: reasoning
  content when the provider emits it (e.g. DeepSeek's
  `reasoning_content`, OpenRouter's `reasoning`), tool calls
  (`[tool] usda_search [...]`), and the model's raw text output before
  parsing. Parse errors per retry print as a standalone red line between
  blocks.

Blocks are always separated by a blank line. When the terminal supports
color (per intake's usual `NO_COLOR` / `CLICOLOR_FORCE` / tty rules), the
markers are bold yellow, request lines cyan, and response lines green —
intake computes this itself and passes it to its renderer; the lib carries
no color setting.

Neither toggle affects behavior; both only add stderr visibility. Nothing
goes to stdout — proposal tables and the confirm dialog are unaffected.
Turning on requests without responses shows what is being sent but nothing
about what comes back, and vice versa; both toggles together reproduce the
full per-round trace.

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

All day-log writes are full-file rewrites (`append_entry`,
`set_exercise_calories`, and the checked `write_day_checked` in
`src/ai/write.rs`, built on `log::write_day_locked`), so a rewrite can
only preserve the fields the binary knows. `DayLog` and `LogEntry` — like
`Food` and `Ingredient` — already carry `#[serde(deny_unknown_fields)]`,
and that is what makes the whole-day rewrites and the `ai food edit`
overwrite safe: a day or food file containing a field this binary doesn't
know fails loudly at load instead of being silently dropped on write.
Adding a log-field in the future is thus a breaking schema change for older
binaries — intentional, since an old binary must never silently delete
data it cannot read; the upgrade path is a migration at the point the
field lands.

`similar` is an intake-crate dependency: proposal rendering is intake's job
(the lib's `present` callback), so the lib stays generic.

## Exit codes

The conventional contract — 0 = "ran as the user intended", non-zero =
failure — with the POSIX `rm(1p)` rule applied to confirmation: exit
status 0 covers work **cancelled by a non-affirmative response to a
prompt**; an error is >0.

| Code | Meaning |
|---|---|
| 0 | Success — the change was written, **or** the user declined (`Reject`) — which covers EOF at the confirm prompt, since closed stdin is a decline; a "Nothing written" line is printed so exit 0 is never a silent no-op |
| 1 | Failure — `ResolveError::Exhausted` / `Internal`, config and file errors (the existing anyhow `?` flow) |
| 2 | Usage error — clap's default on argument-parse errors |
| 130 | Ctrl-C — 128+SIGINT, delivered by the OS; no signal handler is installed, so the default death is kept |

A decline is user-initiated and not an error: answering `n`, or EOF at the
confirm prompt (non-interactive stdin without `--yes`), both exit 0 — EOF
with a stderr note ("no confirmation received — nothing written; use
`--yes` to skip confirmation") so a script piping empty stdin isn't
silently no-op'd. `--yes` is the script's guarantee, like `rm -f`. The
same rule applies to the plain y/n confirm path (`food new` / `food edit`),
so both confirmers behave identically.

## Configuration

```toml
[ai]
model = "..."            # required; or INTAKE_AI_MODEL / --model
base_url = "..."         # required; any OpenAI-compatible endpoint; or INTAKE_AI_BASE_URL / --base-url
api_key = "..."          # or INTAKE_AI_API_KEY / --api-key; optional for endpoints without auth
max_retries = 3
max_tool_calls = 20      # tool executions per resolve attempt; exhaustion → one "answer now" round
timeout_secs = 60        # per LLM API call
usda_api_key = "..."     # or INTAKE_AI_USDA_API_KEY
usda_timeout_secs = 15   # per USDA backend fetch
trace_requests = false   # or --trace-requests; print request messages to stderr
trace_responses = false  # or --trace-responses; print model responses to stderr
history_days = 14        # ai log history window, days before the edited day
log_prompt = "..."       # optional overrides of default templates
food_new_prompt = "..."
food_edit_prompt = "..."
```

- `Config` gets a `#[cfg(feature = "ai")]`-gated `ai: Option<AiConfig>` field
  (see Feature gating), so existing configs and tests are unaffected; the
  `[ai]` table deserializes directly into the intake-side `AiConfig` wrapper
  (`src/ai/settings.rs`), which owns the schema (key allowlist, friendly
  unknown-key errors) as an all-optional table, plus the intake-owned keys
  (`log_prompt`, `food_new_prompt`, `food_edit_prompt`, `history_days`).
  No provider defaults exist anywhere: `model` and `base_url` are required.
- Resolution order matches `Config::resolve`: config file → env var → CLI flag.
  The merge happens in `ai` (`src/ai/mod.rs`): it combines the `[ai]` config
  values, the `INTAKE_AI_*` env vars, and the shared `ai` flags into the
  final `Settings` before constructing a `Resolver` and calling `resolve`.
  `model` and `base_url` must resolve somewhere in that chain — a missing
  value is a friendly error, not a default. `usda_api_key` resolves config →
  env only — no CLI flag.
- The API keys (LLM and USDA) are never logged or printed.
- Timeouts: `timeout_secs` (default 60) bounds each LLM API call;
  `usda_timeout_secs` (default 15) bounds each `usda_search`
  fetch.
  Transient transport failures on the LLM endpoint (HTTP 429/5xx) retry up
  to twice with backoff; timeouts and other errors abort with a clear
  error — no further automatic retry (`max_retries` covers only validation
  failures, not transport failures). USDA fetches never auto-retry: any
  failure, including 429, returns a short error string to the model and
  counts against the tool budget — the model's own retry of a
  `usda_search` call is the throttle, doubling as natural
  rate-limit backoff.
  The editor prompt capture and confirmation prompt are user-paced and
  untimed.
- Known limitation: USDA lookup requires a model/provider that supports
  function/tool calling. If the endpoint rejects `tools`, error clearly and
  suggest a different model.

## Privacy

Running an `ai` command sends data to the configured LLM provider: the
user's prompt; the command's context (`ai log`: the edited day's entries,
the history digest (the `history_days` window, count-deduplicated), totals
and targets; `ai food edit`: the target food's TOML; `ai food new`: the
three sample foods); the queries the model makes via
`usda_search`; and the generated proposal.
`food_lookup` queries are also visible to the provider (they are part of
the conversation), though the results never leave the machine.
`usda_search` additionally sends its queries from intake's
process straight to `api.nal.usda.gov` — the provider is not involved in
that hop. Nothing is sent unless an `ai`
command is run, and nothing is stored beyond intake's existing local
files: the conversation lives in memory only, for the duration of the
resolve.

The API keys travel only where they must: the LLM key in the
`Authorization` header to the configured `base_url`, the USDA key to
`api.nal.usda.gov`; neither is ever logged or printed. The design is
provider-neutral, so users who want the data to never leave their
machine can point `base_url` at a local endpoint (Ollama, vLLM, etc.) —
the same flow then runs fully offline.

## Feature gating

AI support is a Cargo feature on the `intake` binary crate, enabled by
default:

```toml
[features]
default = ["ai"]
ai = ["dep:intake-ai", "dep:serde_json", "dep:similar"]
```

The `serde_json` and `similar` deps ride the same feature because the `ai`
tree's own code needs them — `serde_json` for the `Tool` schemas
(`food_lookup`, USDA), `similar` for the `ai log` row diff — and they must
stay optional so the no-AI build omits them. `similar` is an intake-crate
dependency (proposal rendering is intake's job), never a lib dependency.

- With the default feature, everything in this doc applies.
- Gating is **module-level**, not scattered: `#[cfg(feature = "ai")] mod ai;`
  in main.rs owns the clap args (`src/ai/cli.rs`), the `ai` tree handlers,
  the prompt templates, `INTAKE_AI_*` env resolution, and the Resolver
  wiring for the AI commands — the editor capture and the y/n confirm
  helper are shared, non-gated code (see "Crate boundaries"); no cfg
  attributes inside the module. Every ai-only item lives inside `src/ai/`
  (`cli.rs` clap tree, `settings.rs` config wrapper, `confirm.rs` terminal
  `Confirmer`, `write.rs` checked day writes, `catalog.rs` food listing),
  so shared files carry no per-item attributes. The templates are `.md`
  files under `src/ai/prompts/` (`log_head.md` + `log_tail.md`,
  `food_new_head.md` + `food_new_tail.md`, `food_edit_head.md` +
  `food_edit_tail.md`, `query_style.md`), embedded via `include_str!` *inside*
  the gated module — so they compile
  into the binary only when the `ai` feature is on; no-feature builds embed
  nothing. The plain `food new`/`food edit` editor prefill is a non-gated
  schema skeleton (serialized canned example), not these files.
  The cfg sites are: `#[cfg(feature = "ai")] mod ai;` in main.rs; the
  `Ai` variant on the `Commands` enum in cli.rs (referencing
  `crate::ai::cli::AiCommands`); its match arm in `commands/mod.rs`; and
  the `ai: Option<crate::ai::settings::AiConfig>` field in config.rs — four
  in total, no cfg attributes inside `ai`. (The no-feature test that
  rejects an `[ai]` config table carries the mirrored
  `#[cfg(not(feature = "ai"))]`, and the `ai` e2e test binary is gated via
  `[[test]] required-features = ["ai"]` in Cargo.toml — no attributes.)
- Without the feature, the CLI is the full surface minus the `ai` tree. No
  cross-feature config compatibility is provided: a config.toml containing
  an `[ai]` table fails to parse with the standard unknown-field error
  (`Config` denies unknown fields, and the `ai` field doesn't exist in
  no-feature builds) — the same loud rejection any unrecognized key gets.
  No warning special case, no raw-text preprocessing.
- `intake-ai` is a workspace member, not itself gated: its own code and tests
  build and run regardless of intake's feature selection.

## Prompt templates

Templates are `.md` files under `src/ai/prompts/`, embedded at compile time
via `include_str!` (built into the binary). Each prompt is assembled with
`concat!` around the shared `query_style.md` block — the `usda_search`
query rules (identifying words, batching, and the per-100g scaling rule),
the lookup guidance every session shares since all three tool sets include
`usda_search`. It lives in exactly one place and is spliced into all three
templates. The `food_lookup` query rules are `ai log`-only and live in
`log_head.md`, because `ai log` is the only session with the tool. The
`[ai]` config keys (`log_prompt`,
`food_new_prompt`, `food_edit_prompt`) override only the **static text** —
the context block is always injected by code, appended after the static
text, so an override cannot omit the data the model needs. These files are
for the model only: the
plain `food new` / `food edit` commands prefill the editor from a non-gated
schema skeleton (a serialized canned example with `#` guidance comments)
instead, since they work without the `ai` feature.

Every template has the same anatomy:

1. **Role and task** — one or two sentences framing the command.
2. **Schema** — the exact TOML the model must emit (`Food` + `Ingredient`,
   or `DayLogOps`), mirroring `AGENTS.md` / the serde structs.
3. **Rules** — all macro fields required, no default of zero; the macro
   source order for `ai log` is: the history digest first (an entry matching
   the request is copied and scaled to the portion — an accepted
   scaling exception, estimate-grade), then a tool result where
   one exists (`food_lookup` before going online, then `usda_search` for the
   right variant, scaled to the portion by the model — never recompute or
   estimate from memory when a result exists), then a
   best-effort estimate (the proposal diff shows every value and the user
   confirms before anything is written). Output TOML only, no prose, no
   surrounding fenced blocks.
4. **Examples** — 1-2 short canned examples illustrating the schema and
   conventions; the `log` template embeds its examples inline in the Schema
   section,
   and the history digest doubles as live examples of the user's actual
   patterns. For `food_new`/`food_edit` the context embeds three of the
   user's own foods (see "Context windows"), and the
   canned examples serve as the fallback when the foods dir is empty.
5. **Context block** (code-injected, never part of the `.md`): the
   per-command data from "Context windows", injected last so the user's
   real examples carry the recency weight over the canned ones.

Per-command content:

- `log_head.md` + `log_tail.md` (`ai log`):
  - **Schema** — the `DayLogOps` shape (see "`ai log` ops").
  - **Rows are references** — the numbered context rows are handles the ops
    point at, never entries to re-emit; the model's output is ops only.
  - **Digest before lookup** — prefer scaling values from the history
    digest when available to avoid round-trips; before going online,
    include intended titles in a batched `food_lookup` call, deciding per
    title: `add-food` when a match fits the user's intent (macros computed
    by intake), `add-adhoc` with all six macros for one-offs and modified
    versions — scaled to the portion from a `usda_search` per-100g result
    (`servings = 1`)
    when one exists, else from the history digest scaled to the portion,
    else a best-effort estimate.
  - **Query style** — `food_lookup` takes bare food names: strip portion
    suffixes and quantities ("- 55g", "x 2", "2 cups") from row titles
    before querying and retry with a less specific name on a no-match;
    the `usda_search` rules (identifying words, batching, scaling) come
    from the shared `query_style.md` block.
  - **Row semantics** — `row` indices are 1-based against the numbered list
    exactly as shown and never shift as ops apply (see "`ai log` ops").
- `food_new_head.md` + `food_new_tail.md` (`ai food new <name>`): the `Food` schema; the name is
  supplied on the command line and the title may differ from it (see "Name
  argument"); `notes` is optional.
- `food_edit_head.md` + `food_edit_tail.md` (`ai food edit <name>`): the target food's TOML is the
  context; preserve all untouched fields; the 3 sample foods for
  consistency; before/after is shown at confirm.

## Implementation steps

1. **`crates/intake-ai` + workspace** — root `Cargo.toml` → `[workspace]` with
   `resolver = "2"`, `members = [".", "crates/intake-ai"]`; the `intake`
   package stays at the root (no moves). The new member: settings, usda,
   llm, pipeline, and confirm (trait
   only) modules with unit tests. Text in, validated `T` out; no editor, no
   terminal confirmer impl. Verify all four quality gates stay green at the
   root.
2. **AI integration** — `[features] default = ["ai"]`,
   `ai = ["dep:intake-ai", "dep:serde_json", "dep:similar"]`; `[ai]` config wiring, the
   feature-gated `ai` tree (`ai log`, `ai food new`, `ai food edit`) with the
    Resolver + confirmation flow, the three-option terminal `Confirmer`,
    `--yes` auto-accept (`ResolveContext::auto_accept`), per-command prompt
    template files (`src/ai/prompts/*.md`), the `Tool` trait wiring
    (`usda_search` +
    batched `food_lookup` with per-command registration), the `DayLogOps`
   schema with `apply_ops`, the checked day write
   (`src/ai/write.rs::write_day_checked`), and the stale-write check.
3. **Docs** — AGENTS.md / README `[ai]` section and path touch-ups.

## Testing

- `intake-ai` unit: fence stripping; retry loop via scripted fake backend
  (bad → good TOML in sequence); USDA result parsing from canned JSON
  (search and get responses); per-100g → amount scaling exactness, with an
  absurd `amount_g` returning an error string (checked math, never a
  panic); 429/rate-limit and fetch-failure error strings; batch mode;
  tool-call execution roundtrip; multiple `tool_calls` in one response each
  count against the budget and all results feed back; tool budget
  exhaustion — scripted backend emitting tool calls past `max_tool_calls`
  sees exactly one budget-exhausted message, the next response is taken
  unconditionally (even when it still names tools), and parsing proceeds;
  failed/timed-out calls count against the budget; confirmation decision
  mapping (a scripted
  `Confirmer` impl driving the loop); timeout aborts with a clear error
  (hang-mode fake backend + tiny timeout, no slow tests); transport retry
  (scripted 429/5xx then success, no slow tests); tracing —
  scripted fake backend responses carrying `reasoning_content` produce the
  expected stderr trace lines under `trace_responses`, and the
  `trace_requests` path prints role-prefixed lines for every message sent
  per round; the two toggles are independent (each exercisable alone).
- `intake` unit: the three-option confirmer decision mapping (in the `ai` build);
  exit-code mapping — decline and EOF-at-prompt (a decline) exit 0,
  `Exhausted` and `Internal` exit 1 (both confirmers);
  `--yes` skips proposal rendering;
  `[ai]` config parsing and env/flag resolution;
  `write_day_checked` roundtrip;
  clap parsing of the `ai` tree and shared flags (the shared `--date` arg,
  no short form, `--prompt` vs. `[prompt...]` conflict);
  `apply_ops` (add-food / add-adhoc / remove / replace; row out of
  range; duplicate and conflicting ops; additions append, replacements keep
  position; overflow — huge `servings` returns a retryable `Err(String)`,
  never a panic or `Internal`);   `food_lookup` matching (tiered integer scoring — exact, token overlap,
  bigram Dice; top-N ordering; batch mode — multiple
  queries in one call; empty result for unknown foods and for an empty
  foods dir); context assembly (totals line, history digest anchored to the
  edited day, dedup + occurrence counts, count sort order, pre-dedup cap,
  empty window, sample foods embedded for
  `ai food new`/`edit` (diversity slots: complex / notes / simple, slot
  fallback, catalog with <3 foods, empty-dir canned fallback); stale-write abort (target changed
  between context build and write); empty ops proposal (empty diff, no
  write); name argument — parse-time collision error for `ai food new`,
  suggesting `food edit`;
  write-time recheck for `ai food new`;
  row diff (one macro
  changed → exactly one `-`/`+` pair; entry added → one `+` line; unchanged →
  empty diff); no-feature build rejects a config containing an `[ai]` table
  with the standard unknown-field error (feature off);
  prompt-file drift guard (each prompt `.md` under `src/ai/prompts/` contains
  its key schema field names, e.g. `protein_g` / `add-adhoc` — fails if serde
  structs change without prompt updates).
- `intake` e2e (`tests/ai.rs`, compiled only with the `ai` feature via
  `[[test]] required-features` in Cargo.toml — no cfg attributes in the
  shared file): the real binary driven against a fake OpenAI-compatible
  server (local TCP listener serving scripted `chat/completions`
  responses), covering `ai log` with confirm-yes (proposal diff shown
  once, day written, one-line confirmation instead of a second table),
  confirm-no (nothing written), and `--yes` (no confirmation; the day
  table is the only display), plus `ai food new` in `--yes` (food table
  as the only display, no confirmation line) — including assertions on
  the recorded request (model name, `food_lookup` tool advertised, prompt
  carried).
- Quality gates (workspace root): `cargo test --workspace`,
  `cargo clippy --workspace -- -D warnings`,
  `cargo fmt --check`, `cargo build --workspace`, plus `cargo test -p intake
  --no-default-features` and `cargo build -p intake --no-default-features`
  for the no-AI configuration. The `--workspace` flags are required because
  this is a non-virtual workspace (the root is a package): bare `cargo
  test` from the root runs the `intake` package only, which would leave
  `intake-ai`'s own tests and lints uncovered. The no-AI gates are scoped
  to the `intake` package so `intake-ai`'s dependencies stay out of the
  no-AI build. `intake-ai` keeps its coverage through the workspace-wide
  gates; when it is split off into its own repository, its `cargo test`
  runs everything there and the `--workspace` flags here go away.

## Docs

Update `AGENTS.md` and `README.md` with the `[ai]` config section, a note on
the USDA FoodData Central-backed nutrition lookup (`usda_search`),
and corrected paths for the workspace layout.

## Open questions / future work

- Keyed search backends (Brave, Tavily) and self-hosted SearXNG.
- Generic `web_search` + `fetch_url` as the fallback for USDA gaps
  (restaurant meals, branded oddities, non-US foods): search returning
  candidate URLs, plus a full-page fetch tool (capped HTML→text) so the
  model reads real nutrition tables rather than ~200-char snippets.
  Deferred from v1: snippets cannot carry all six macros plus serving
  context, the model must extract from arbitrary page layouts, and
  raw/cooked + serving ambiguity resolves only on the page, not in the
  snippet. 2-3 calls per item (vs 1-2 batched with USDA), so
  `max_tool_calls`/batching needs rework when it lands. A macro agent
  (`estimate_macros`, below) is the natural pairing for this backend.
- A specialized macro agent exposed to the main agent as a tool
  (`estimate_macros`): a nested `resolve` (one new template file + a `Tool`
  impl running a sub-loop) that takes a food description + portion and
  returns exact per-serving macros — fixes the arithmetic-scaling and
  macro-completeness error classes and isolates the main agent's tool
  budget. Without `usda_get`, scaling is model-side for all tool-sourced
  data, so the agent's value is concentrated in the `web_search` era
  (extraction from arbitrary pages). Details not scoped: caching, output
  contract/confidence, nested budget accounting, and whether it rides on
  `web_search` from day one.
- Templating the template: variables/placeholders inside the prompt `.md`
  files (context blocks are currently injected by code only, so overrides
  cannot reach them).
- Streaming token output from the model (independent of tracing, which
  works without streaming).
- If `ai log` proves slower or less reliable than expected for simple appends,
  an add-only mode (no remove/replace ops, no day context or diff) is a thin
  variant.
- A `log_lookup` tool as the sibling of `food_lookup` (given a date or range,
  return the day logs as entry lines) — would later extend the configurable
  history digest, letting the model pull older logs on demand; returning full
  macro values would also let intake scale them code-side, closing the
  remaining model-side scaling gap in "Context windows".
- Optionally, search tools for each domain on top of lookup: a `food_search`
  (fuzzy/full-text search across food titles, ingredients, and notes) and a
  `log_search` (search past log entries by title or macros, e.g. "when did I
  last log oatmeal") — useful once catalogs and histories outgrow what a
  lookup-by-name or fixed window can cover. `food_search` would serve the
  recipe builders (`ai food new`/`edit`) once their catalogs outgrow the
  few embedded sample foods.
- An optional `exercise_calories` op in `ai log` (v1: preserved structurally
  by `apply_ops`, unchangeable).
- Publishing `intake-ai` to crates.io as part of the split-off.

