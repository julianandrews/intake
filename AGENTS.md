# intake

This project is a CLI diet tracker. Use the `intake` binary for all operations —
run `intake --help` to see available subcommands and flags.

## Adding a Food

Create a `.toml` file in your foods directory (`~/.local/share/intake/foods/`
by default). The filename (minus `.toml`) becomes the food slug.

The file must match the `Food` and `Ingredient` structs in `src/food.rs`:

- `title` — display name
- `servings` — how many servings the full food makes
- `notes` (string, optional — defaults to empty; shown under the recipe in
  `intake show` only when non-empty)
- `[[ingredients]]` — one table per ingredient, each with:
  - `name` (string, required)
  - `quantity` (string, optional — e.g. `"400g"`, `"1 tbsp"`)
  - `protein_g` (float or int)
  - `fiber_g` (float or int)
  - `calories` (int)
  - `fat_g` (float or int)
  - `carbs_g` (float or int)
  - `alcohol_g` (float or int)

All macro fields (`protein_g`, `fiber_g`, `calories`, `fat_g`, `carbs_g`,
`alcohol_g`) are **required** — there is no default of zero, so a food with
unrecorded macros fails loudly instead of silently counting as zero.

See `tests/fixtures/foods/cheesy-popcorn.toml` for a simple example, or
`tests/fixtures/foods/turkey-chili.toml` for a multi-ingredient one.

The `content_hash` field is computed automatically at load time — do not include it.

## Adding an Adhoc Log Entry

Use the `adhoc` CLI subcommand for one-off foods without a food file:

```
intake adhoc [--calories N] [--protein N] [--fiber N] [--fat N] [--carbs N] [--alcohol N] <name> [servings]
```

- Macros are specified inline — no food file needed; every macro flag is
  optional and defaults to 0
- The entry is appended to today's log
- Slug is auto-derived from the name (lowercased, spaces → hyphens)
- Check existing entries in `log/` for the exact TOML format to follow
  (especially adhoc entries with a `title` field)
- Implementation: `src/main.rs` function `cmd_adhoc`
- When you need to look up macros for a food, use `websearch` to find reliable
  nutrition data (e.g. from the USDA). Prefer data per 100g and scale by the
  serving size.

## Configuration

The config file lives at `~/.config/intake/config.toml`. Data defaults to
`~/.local/share/intake/` (following XDG_DATA_HOME). Supported fields:

- `foods_dir` / `log_dir` — override default data directories
- `show_columns` — which macro columns to display in `log`/`summary`/`list`/`show`.
  Values: `calories`, `protein`, `fiber`, `fat`, `carbs`, `alcohol`.
  Default: all except `alcohol`. Duplicate entries are a config error
- `max_calories` — daily calorie target (u32)
- `min_protein` — daily protein target in grams (f64)
- `min_fiber` — daily fiber target in grams (f64)
- `maintenance_calories` — TDEE for deficit calculation (u32)
- Every macro also accepts `min_<macro>` / `max_<macro>` targets (f64), e.g.
  `min_fat`, `max_fat`, `min_carbs`, `max_carbs`, `max_alcohol`. When both a
  min and max are set the `[min, max]` range is the green band: below min →
  yellow, above max → red, inside → green. Targets scale with day progress
  (like `max_calories` does). Coloring logic: `column_color` in `src/display.rs`

Paths can also be set via `INTAKE_FOODS_DIR` / `INTAKE_LOG_DIR` env vars, or
`--foods-dir` / `--log-dir` CLI flags. Resolution order:
config file → env var → CLI flag.

The `Config` struct and its resolution logic live in `src/config.rs`.

## Exercising

```
intake exercise 300
```

Records calories burned for today. On exercise days the log table shows an
`Exercise` row with a negative calorie adjustment and a `Net` row with
post-exercise calories; exercise also raises the TDEE used for deficit
calculation. If `show_columns` omits `calories`, the `Exercise`/`Net` rows are
hidden (they have nothing to show); the summary command likewise hides its
`Exercise` column in that case.

## Viewing a Multi-Day Summary

`intake summary [date] [--days N]` (`-d` shorthand) shows one row per logged
day (unlogged days in the window are skipped) with total macros, exercise, and
deficit per day, plus period totals and per-day averages. The Deficit column
appears only when `maintenance_calories` is configured. Implementation:
`src/main.rs` functions `cmd_summary` and `build_summary_rows`.

## Code Quality

Always run these checks before committing or finishing a task:

1. **`cargo test`** — all tests must pass
2. **`cargo clippy -- -D warnings`** — no clippy warnings (deny all)
3. **`cargo fmt --check`** — formatting must match `rustfmt`
4. **`cargo build`** — clean build with no warnings

Run them in the project root.
