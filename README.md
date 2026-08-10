# intake

A CLI diet tracker. Log what you eat and track macros against daily targets.

## Quick Start

```sh
cargo build --release
intake --help
```

### Set up shell completions

```sh
intake completions bash --install
source ~/.local/share/bash-completion/completions/intake
```

Or for zsh: `intake completions zsh --install`

## Usage

```
intake                                  Show today's log
intake log <name> [servings] [--date D]     Log a food (macros from its file)
intake log "<name>" [servings] --calories N --protein N --fiber N --fat N --carbs N --alcohol N
                                          Log an ad-hoc entry with inline macros
intake day [date] [-d N]                Show a day's totals (default: today)
intake summary [date] [-d N]            Multi-day summary of macros and deficit (default: last 7 days)
intake exercise <calories>              Record exercise calories for today
intake rm <n> [--date D]                Remove an entry from a day's log
intake food list                        List all foods with per-serving values
intake food show <name>                 Show food details with ingredients
intake food new <name>                  Create a food in your editor
intake food edit <name>                 Edit a food in your editor
intake food rm <name>                   Delete a food (existing log entries are unaffected)
intake ai log [prompt...] [--date D]    Edit a day's log with AI (ops-based)
intake ai food new <name> [prompt...]   Create a recipe with AI
intake ai food edit <name> [prompt...]  Edit a recipe with AI
intake completions <shell>              Generate or install completion script
```

Flags like `--foods-dir` and `--log-dir` are available on every command.
`intake day --days-ago N` (or `-d N`) shows the log from N days ago, e.g.
`intake day -d 1` for yesterday. `intake log <name> --date D` logs to day D
instead of today. `intake summary` shows one row per logged day (unlogged
days in the window are skipped) with period totals and per-day averages;
the averages and totals are over the logged days only, not the full window
length. The Deficit column appears when `maintenance_calories` is configured.

`intake day` numbers its rows (the `#` column); `intake rm <n> --date D`
removes that entry (default: today's log) after a confirmation prompt,
`--yes` skips it. Removing the last entry of a day that has no exercise
calories deletes the day file itself, so `intake day` reports "No entries".

Bare `intake` with no subcommand shows today's log.

## Configuration

Create `~/.config/intake/config.toml` to set daily targets:

```toml
max_calories = 1800
min_protein = 150.0
min_fiber = 30.0
min_fat = 50.0
max_fat = 90.0
maintenance_calories = 2400
show_columns = ["calories", "carbs", "fat", "protein", "fiber"]
foods_dir = "/path/to/foods"
log_dir = "/path/to/logs"
```

`show_columns` controls which macro columns appear in `day`, `summary`,
`food list`, and `food show` (values: `calories`, `protein`, `fiber`, `fat`,
`carbs`, `alcohol`; default: all except `alcohol`; duplicate entries are
rejected). Every macro accepts `min_<macro>` / `max_<macro>` targets — with
both set, the `[min, max]` range is the green band: below min is yellow,
above max is red. Targets scale with day progress.

Paths can also be set via `INTAKE_FOODS_DIR` and `INTAKE_LOG_DIR` environment
variables, or `--foods-dir` / `--log-dir` CLI flags (CLI wins).

## AI Commands

`ai log`, `ai food new`, and `ai food edit` route a prompt through an LLM,
show the proposed change, and write it only after your confirmation
(`--yes` skips the proposal and confirmation). The prompt can be given as a
positional, via `--prompt "..."`, or by opening `$VISUAL` / `$EDITOR` /
`nano` on a temp file (leave it unchanged to abort). Nutrition data comes
from the USDA FoodData Central API via the `usda_search` / `usda_get`
tools; `ai log` also looks up your own foods (`food_lookup`). Anything a
model returns is validated and re-requested on failure, and the day or food
file is re-checked before writing so a concurrent change aborts instead of
being overwritten.

```sh
intake ai log "add a late snack under 200 cal"
intake ai food new tjs-lunch "build a lunch like my usual one"
intake ai food edit turkey-chili "make it lower sodium"
```

Configure the provider in `~/.config/intake/config.toml`:

```toml
[ai]
api_key = "..."          # or INTAKE_AI_API_KEY / --api-key
model = "gpt-4o-mini"    # or INTAKE_AI_MODEL / --model
# base_url = "https://api.openai.com/v1"   # any OpenAI-compatible endpoint
usda_api_key = "..."     # or INTAKE_AI_USDA_API_KEY (free at fdc.nal.usda.gov)
history_days = 14        # ai log context window
```

Settings resolve config file → environment → CLI flags (`--api-key`,
`--model`, `--yes`, `--trace-requests`, `--trace-responses`, `--prompt`).
Tracing is off by default; `--trace-requests` prints each message sent to
the model once, on the round it first appears (stderr, in `--- to model ---`
blocks), and `--trace-responses` prints the model's output in
`--- from model ---` blocks (both
also settable as `[ai] trace_requests` / `[ai] trace_responses`; blocks are
colorized when the terminal supports it). Without a
`usda_api_key` the USDA tools error with a
setup hint; pointing `base_url` at a local endpoint (Ollama, vLLM, ...)
keeps the whole flow offline. Running an `ai` command sends the prompt,
context, and tool queries to the configured provider; the conversation is
never stored.

## Adding Foods

Drop a `.toml` file in your foods directory, or run `intake food new <name>`
to create one in your editor (it pre-fills a template with guidance
comments; `$VISUAL`, `$EDITOR`, or `nano` is used, and `--yes` skips the
confirmation prompt). `intake food edit <name>` edits an existing food the
same way. The filename (minus `.toml`) becomes the food name used with
`intake log`. The editor value is split on whitespace, so `code --wait`
works but quoted arguments are not supported.

```toml
title = "My Food"
servings = 4

[[ingredients]]
name = "Chicken"
quantity = "200g"
protein_g = 46.0
fiber_g = 0.0
calories = 330
fat_g = 6.0
carbs_g = 0.0
alcohol_g = 0.0
```

All macro fields (`protein_g`, `fiber_g`, `calories`, `fat_g`, `carbs_g`,
`alcohol_g`) are required — a food with unrecorded macros fails loudly
instead of silently counting as zero.

`intake food show <name>` reports parse errors, while `intake food list`
skips food files that fail to parse, printing a warning to stderr for each
one.

`intake food rm <name>` deletes a food file (with a confirmation prompt;
`--yes` skips it). Existing log entries are standalone copies of a food's
values, so removing a food never changes or breaks earlier logs.

See `tests/fixtures/foods/` in the repo for examples.

## Adhoc Entries

For one-off foods without a food file, pass macro flags to `log` — any macro
flag selects the ad-hoc path, and the name is the entry's title:

```
intake log --calories 250 --protein 12 --fiber 3 --fat 9 --carbs 20 "Greek yogurt" 1.5
```

All macro flags (`--calories`, `--protein`, `--fiber`, `--fat`, `--carbs`,
`--alcohol`) are optional and default to 0. Without macro flags the name
must be an existing food name; anything else errors instead of logging a
silently-zero entry — for a zero-calorie item (e.g. water), pass `--calories 0`.

The name is stored verbatim as the entry's title — no food file or name is
created.

## Log Files

Each day is stored as `YYYY-MM-DD.toml` in the log directory. Parsing is
strict: like food files, unknown fields or malformed values in a day log
error loudly instead of being ignored.
