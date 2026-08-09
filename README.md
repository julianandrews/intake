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
intake log <slug> [servings] [--date D]     Log a food (macros from its file)
intake log "<name>" [servings] --calories N --protein N --fiber N --fat N --carbs N --alcohol N
                                          Log an ad-hoc entry with inline macros
intake day [date] [-d N]                Show a day's totals (default: today)
intake summary [date] [-d N]            Multi-day summary of macros and deficit (default: last 7 days)
intake exercise <calories>              Record exercise calories for today
intake food list                        List all foods with per-serving values
intake food show <slug>                 Show food details with ingredients
intake food new <slug>                  Create a food in your editor
intake food edit <slug>                 Edit a food in your editor
intake completions <shell>              Generate or install completion script
```

Flags like `--foods-dir` and `--log-dir` are available on every command.
`intake day --days-ago N` (or `-d N`) shows the log from N days ago, e.g.
`intake day -d 1` for yesterday. `intake log <slug> --date D` logs to day D
instead of today. `intake summary` shows one row per logged day (unlogged
days in the window are skipped) with period totals and per-day averages; the
Deficit column appears when `maintenance_calories` is configured.

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

## Adding Foods

Drop a `.toml` file in your foods directory, or run `intake food new <slug>`
to create one in your editor (it pre-fills a template with guidance
comments; `$VISUAL`, `$EDITOR`, or `nano` is used, and `--yes` skips the
confirmation prompt). `intake food edit <slug>` edits an existing food the
same way. The filename (minus `.toml`) becomes the food slug used with
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

`intake food show <slug>` reports parse errors, while `intake food list`
skips food files that fail to parse, printing a warning to stderr for each
one.

See `tests/fixtures/foods/` in the repo for examples.

## Adhoc Entries

For one-off foods without a food file, pass macro flags to `log` — any macro
flag selects the ad-hoc path, and the name is the entry's title:

```
intake log --calories 250 --protein 12 --fiber 3 --fat 9 --carbs 20 "Greek yogurt" 1.5
```

All macro flags (`--calories`, `--protein`, `--fiber`, `--fat`, `--carbs`,
`--alcohol`) are optional and default to 0. Without macro flags the name
must be an existing food slug; anything else errors instead of logging a
silently-zero entry — for a zero-calorie item (e.g. water), pass `--calories 0`.

The name is stored verbatim as the entry's title — no food file or slug is
created.

## Log Files

Each day is stored as `YYYY-MM-DD.toml` in the log directory. Parsing is
strict: like food files, unknown fields or malformed values in a day log
error loudly instead of being ignored.
