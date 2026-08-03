# intake

A CLI diet tracker. Log what you eat and track macros.

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
intake completions <shell>          Generate completion script
```

## Adding Recipes

Drop a `.toml` file in `foods/`. Look through other files in `foods` for examples.

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
