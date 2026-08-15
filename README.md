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
intake [--date D | --days-ago N]            Show a day's totals (default: today)
intake log <name> [servings] [--time HH:MM] [--date D | --days-ago N]
                                              Log a food (macros from its file)
intake log "<name>" [servings] --calories N --protein N --fiber N --fat N --carbs N --alcohol N [--time HH:MM]
                                              Log an ad-hoc entry with inline macros
intake summary [--date D | --days-ago N] [--days N]
                                              Multi-day summary of macros and deficit (default: config summary_days, or 7 days)
intake exercise <calories> [--date D | --days-ago N]
                                              Record exercise calories for a day (default: today)
intake rm <n> [--date D | --days-ago N]       Remove an entry from a day's log
intake retime <n> <HH:MM> [--yes] [--date D | --days-ago N]
                                              Set an entry's timestamp
intake food list                              List all foods with per-serving values
intake food show <name>                       Show food details with ingredients
intake food new <name>                        Create a food in your editor
intake food edit <name>                       Edit a food in your editor
intake food rm <name>                         Delete a food (existing log entries are unaffected)
intake ai log [prompt...] [--date D | --days-ago N]
                                              Edit a day's log with AI (ops-based)
intake ai food new <name> [prompt...]         Create a recipe with AI
intake ai food edit <name> [prompt...]        Edit a recipe with AI
intake completions <shell>                    Generate or install completion script
```
Flags like `--foods-dir` and `--log-dir` are root-level: accepted before
the subcommand on every invocation.
`--date D` and `--days-ago N` (or `-d N`) target a day the same way on every
date-targeting command — bare `intake`, `log`, `exercise`, `rm`, `ai log`,
and `summary` (where `--days-ago` positions the end of the window). Omitting
both targets today; supplying both is an error. Date flags on a subcommand
win over date flags on the bare command, and with a non-date command like
`food` they are an error. For example, `intake --days-ago 1` shows
yesterday, `intake log coffee -d 1` logs coffee to yesterday, and
`intake --days-ago 1 log coffee` does the same with the flag before the
command, and `intake exercise 300 -d 2` records exercise two days ago.
`intake summary` shows one row per logged day (unlogged days in the window
are skipped) with period totals and per-day averages; the averages and
totals are over the logged days only, not the full window length. The
window defaults to 7 days, overridable with `summary_days` in config or
`--days` on the command line (CLI wins). The Deficit column appears when
`maintenance_calories` is configured.

The day view numbers its rows (the `#` column); `intake rm <n> --date D`
removes that entry (default: today's log) after a confirmation prompt,
`--yes` skips it. Removing the last entry of a day that has no exercise
calories deletes the day file itself, so the day view reports "No entries".

Every entry carries a timestamp recording when it was logged (a full RFC
3339 string in UTC, shown in the day view's Time column in local time).
`intake log coffee --time 14:30 --days-ago 1` stamps the entry at 14:30 on
the target date instead of now — handy when you can't log a meal until
later; `intake retime 2 14:30` adjusts an entry that's already logged
(1-based row number, confirmation prompt, `--yes` skips it). See the
Configuration section for the `write_timestamps`, `show_timestamp`, and
`time_format` keys.

Bare `intake` with no subcommand shows today's log.

Changed: the `day` subcommand is gone — the day view is bare `intake`, and
its old `[date]` positional and `-d` form are now `--date D` and `--days-ago
N` on the target command. Also, `summary -d N` no longer sets the window
length (that's `--days N`); `-d`/`--days-ago` is the window's end date,
matching every other date-targeting command.

## Configuration

Create `~/.config/intake/config.toml` to set daily targets:

```toml
max_calories = 1800
min_protein = 150.0
min_fiber = 30.0
min_fat = 50.0
max_fat = 90.0
maintenance_calories = 2400
summary_days = 7          # default window length for summary (default: 7)
show_columns = ["calories", "carbs", "fat", "protein", "fiber"]
write_timestamps = true    # write a timestamp on new entries (default: true)
show_timestamp = true      # Time column in the day view (default: true)
time_format = "24h"        # "24h" → 14:05 (default) | "12h" → 2:05 PM
foods_dir = "/path/to/foods"
log_dir = "/path/to/logs"
```

`show_columns` controls which macro columns appear in the day view,
`summary`, `food list`, and `food show` (values: `calories`, `protein`,
`fiber`, `fat`, `carbs`, `alcohol`; default: all except `alcohol`; duplicate
entries are rejected). Every macro accepts `min_<macro>` / `max_<macro>`
targets — with both set, the `[min, max]` range is the green band: below min
is yellow, above max is red. Targets scale with day progress.

`write_timestamps` controls whether new entries get a timestamp (default
`true`); an explicit `--time` on `log` or the `retime` command always writes
one, regardless of the flag. `show_timestamp` adds the Time column to the
day view (default `true`); `time_format` selects 24-hour `HH:MM` (default)
or 12-hour `h:mm AM/PM` rendering. Entries without timestamps (written
before the feature, or with `write_timestamps = false`) show an empty Time
cell.

Paths can also be set via `INTAKE_FOODS_DIR` and `INTAKE_LOG_DIR` environment
variables, or `--foods-dir` / `--log-dir` CLI flags (CLI wins).

## AI Commands

`ai log`, `ai food new`, and `ai food edit` route a prompt through an LLM,
show the proposed change, and write it only after your confirmation
(`--yes` skips the proposal and confirmation). The prompt can be given as a
positional, via `--prompt "..."`, or by opening `$VISUAL` / `$EDITOR` /
`nano` on a temp file (leave it unchanged to abort). Nutrition data comes
from the USDA FoodData Central API via the `usda_search`
tool; `ai log` also looks up your own foods (`food_lookup`). Anything a
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
model = "..."            # required; or INTAKE_AI_MODEL / --model
base_url = "..."         # required; any OpenAI-compatible endpoint; or INTAKE_AI_BASE_URL / --base-url
api_key = "..."          # or INTAKE_AI_API_KEY / --api-key; optional for local endpoints without auth
usda_api_key = "..."     # or INTAKE_AI_USDA_API_KEY (free at fdc.nal.usda.gov)
history_days = 14        # ai log context window
```

Settings resolve config file → environment → CLI flags (`--api-key`,
`--model`, `--base-url`, `--yes`, `--trace-requests`, `--trace-responses`,
`--prompt`). `model` and `base_url` are required — an `ai` command errors
with a setup hint until both are set. Tracing is off by default;
`--trace-requests` prints each message sent to
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
