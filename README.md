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
intake add <food> [servings]          Add a food to today's log
intake log [date] [--days-ago N]     Show a day's totals (default: today)
intake summary [date] [--days N]     Multi-day summary of macros and deficit (default: last 7 days)
intake show <food>                   Show food details with ingredients
intake list                          List all foods
intake adhoc [--calories N] [--protein N] [--fiber N] [--fat N] [--carbs N] [--alcohol N] <name> [servings]   Log a one-off food
intake exercise <calories>           Record exercise calories for today
intake completions <shell>           Generate or install completion script
```

Flags like `--foods-dir` and `--log-dir` are available on every command.
`intake log --days-ago N` (or `-d N`) shows the log from N days ago, e.g.
`intake log -d 1` for yesterday. `intake summary` shows one row per logged day
(unlogged days in the window are skipped) with period totals and per-day
averages; the Deficit column appears when `maintenance_calories` is configured.

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

`show_columns` controls which macro columns appear in `log`, `summary`,
`list`, and `show` (values: `calories`, `protein`, `fiber`, `fat`, `carbs`,
`alcohol`; default: all except `alcohol`; duplicate entries are rejected).
Every macro accepts `min_<macro>` / `max_<macro>` targets — with both set, the
`[min, max]` range is the green band: below min is yellow, above max is red.
Targets scale with day progress.

Paths can also be set via `INTAKE_FOODS_DIR` and `INTAKE_LOG_DIR` environment
variables, or `--foods-dir` / `--log-dir` CLI flags (CLI wins).

## Adding Foods

Drop a `.toml` file in your foods directory. The filename (minus `.toml`)
becomes the food slug used with `intake add`.

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

See `tests/fixtures/foods/` in the repo for examples.

## Adhoc Entries

For one-off foods without a food file:

```
intake adhoc --calories 250 --protein 12 --fiber 3 --fat 9 --carbs 20 "Greek yogurt" 1.5
```

All macro flags (`--calories`, `--protein`, `--fiber`, `--fat`, `--carbs`,
`--alcohol`) are optional and default to 0.

The name is stored verbatim as the entry's title — no food file or slug is
created.
