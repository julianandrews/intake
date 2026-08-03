# intake

A CLI diet tracker. Log what you eat, track macros, and find recipe combinations
to hit your daily goals.

## Quick Start

```sh
cargo build
export PATH="/workspace:$PATH"
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
intake add <recipe> [servings]     Add a recipe to today's log
intake today                        Show today's totals
intake log 2026-08-01               Show a specific day
intake show <recipe>                Show recipe details
intake list                         List all recipes
intake adhoc --cal N --prot N --fib N <name>   Log a one-off food
intake fill --remaining             Find combos to meet remaining goals
intake completions <shell>          Generate completion script
```

## Adding Recipes

Drop a `.toml` file in `foods/`. See `foods/cheesy-popcorn.toml` for an example.

```
title = "My Recipe"
servings = 4

[[ingredients]]
name = "Chicken"
quantity = "200g"
protein_g = 46.0
fiber_g = 0.0
calories = 330
```

## Configuration

Edit `intake-config.toml` to set daily goals:

```toml
[goals]
max_calories = 1800
min_protein = 150
min_fiber = 30

[search]
max_nodes = 100000
max_results = 1000
```

## Project Structure

```
intake/
├── Cargo.toml           # Rust project
├── src/                 # Source code
├── foods/               # Recipe TOML files
├── log/                 # Daily log files
├── intake-config.toml   # Goals and search config
└── AGENTS.md            # AI assistant instructions
```
