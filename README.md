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
intake add <recipe> [servings]       Add a recipe to today's log
intake log [date]                    Show a day's totals (default: today)
intake show <recipe>                 Show recipe details with ingredients
intake list                          List all recipes
intake adhoc --cal N --prot N --fib N <name> [servings]   Log a one-off food
intake exercise <calories>           Record exercise calories for today
intake completions <shell>           Generate or install completion script
```

Flags like `--foods-dir` and `--log-dir` are available on every command.
Use `--grouped` with `intake log` to merge entries with the same recipe slug.

## Configuration

Create `~/.config/intake/config.toml` to set daily targets:

```toml
max_calories = 1800
min_protein = 150.0
min_fiber = 30.0
maintenance_calories = 2400
foods_dir = "/path/to/recipes"
log_dir = "/path/to/logs"
```

Paths can also be set via `INTAKE_FOODS_DIR` and `INTAKE_LOG_DIR` environment
variables, or `--foods-dir` / `--log-dir` CLI flags (CLI wins).

## Adding Recipes

Drop a `.toml` file in your foods directory. The filename (minus `.toml`)
becomes the recipe slug used with `intake add`.

```toml
title = "My Recipe"
servings = 4

[[ingredients]]
name = "Chicken"
quantity = "200g"
protein_g = 46.0
fiber_g = 0.0
calories = 330
```

See `foods/` in the repo for examples. The `content_hash` is computed
automatically at load time — do not include it.

## Adhoc Entries

For one-off foods without a recipe file:

```
intake adhoc --calories 250 --protein 12 --fiber 3 "Greek yogurt" 1.5
```

The slug is auto-derived from the name (lowercased, spaces → hyphens).
