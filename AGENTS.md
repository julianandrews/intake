# intake

This project is a CLI diet tracker. Use the `intake` binary for all operations —
run `intake --help` to see available subcommands and flags.

## Adding a Food

Create a `.toml` file in your foods directory (`~/.local/share/intake/foods/`
by default). The filename (minus `.toml`) becomes the food slug.

The file must match the `Food` and `Ingredient` structs in `src/food.rs`:

- `title` — display name
- `servings` — how many servings the full food makes; must be a nonzero
  integer (`NonZeroU32`)
- `notes` (string, optional — defaults to empty; shown under the recipe in
  `intake show` only when non-empty)
- `[[ingredients]]` — one table per ingredient, each with:
  - `name` (string, required)
  - `quantity` (string, optional — e.g. `"400g"`, `"1 tbsp"`)
  - `protein_g` (decimal number)
  - `fiber_g` (decimal number)
  - `calories` (decimal number, whole or fractional)
  - `fat_g` (decimal number)
  - `carbs_g` (decimal number)
  - `alcohol_g` (decimal number)

All macro fields (`protein_g`, `fiber_g`, `calories`, `fat_g`, `carbs_g`,
`alcohol_g`) are **required** — there is no default of zero, so a food with
unrecorded macros fails loudly instead of silently counting as zero.

Macro amounts are stored as exact decimals rounded to 0.001 (0.001 g for
masses via `Grams` in `src/amount.rs`, 0.001 kcal for calories via
`Calories`): sums and products are exact, per-serving division rounds to
0.001, and display rounds to 0.1 g / whole calories.
Per-serving calories are *not* rounded to whole numbers — a 100 kcal food
with 3 servings logs 33.333 kcal/serving, keeping day totals within a
rounding hair of exact. Values are written to log files as bare decimal
literals (integers when integral, floats otherwise) — no quotes, and exact
round-trip for values below ≈2.2×10^12 at full 0.001 precision (~15
significant digits; any realistic diet quantity). Values too large for
exact f64 round-trip — and integral values beyond the TOML integer range —
are rejected at serialization, so writing never loses precision silently.
Quoted strings are rejected on read; bare integers and floats are normalized
to exact decimals at the parse boundary; internally everything is decimal.
Food and log arithmetic is checked: overflow in ingredient sums, serving
products, and day or period totals fails loudly with an error instead of
panicking or wrapping. `servings = 0` is rejected
at load — it previously caused silent inf/NaN. Log entries store a strictly
positive decimal `servings` (`Servings`), so fractional servings (0.5, 1.5)
are fine but zero/negative are rejected on load.

See `tests/fixtures/foods/cheesy-popcorn.toml` for a simple example, or
`tests/fixtures/foods/turkey-chili.toml` for a multi-ingredient one.

The `content_hash` field is computed automatically at load time — do not include it.

## Adding an Adhoc Log Entry

Use the `adhoc` CLI subcommand for one-off foods without a food file:

```
intake adhoc [--calories N] [--protein N] [--fiber N] [--fat N] [--carbs N] [--alcohol N] <name> [servings]
```

- Macros are specified inline — no food file needed; every macro flag is
  optional and defaults to 0. Calorie values are decimals (e.g. `--calories 33.333`)
- The entry is appended to today's log
- Check existing entries in `logs/` for the exact TOML format to follow:
  every entry stores its display name in a `title` field (required, on disk
  as a plain string), plus `servings` and the six macros — food and adhoc
  entries have the same shape, and logs render without reading the foods
  directory
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
- `max_calories` — daily calorie target (decimal, `Calories`)
- `min_protein` — daily protein target in grams (decimal)
- `min_fiber` — daily fiber target in grams (decimal)
- `maintenance_calories` — TDEE for deficit calculation (decimal, `Calories`)
- `min_calories` — also accepted (decimal); pairs with `max_calories` as a
  `[min, max]` band like the macros below
- Every macro also accepts `min_<macro>` / `max_<macro>` targets (decimal), e.g.
  `min_fat`, `max_fat`, `min_carbs`, `max_carbs`, `max_alcohol`. When both a
  min and max are set the `[min, max]` range is the green band: below min →
  yellow, above max → red, inside → green. Targets scale with day progress
  (like `max_calories` does). Coloring logic: `column_color` in `src/display.rs`
- Calorie targets are typed as `Calories` (non-negative decimal, rounded to
  0.001 like macros); legacy integer config values still parse. Negative
  calorie targets (`min_calories`, `max_calories`, `maintenance_calories`)
  are rejected at parse

Paths can also be set via `INTAKE_FOODS_DIR` / `INTAKE_LOG_DIR` env vars, or
`--foods-dir` / `--log-dir` CLI flags. Resolution order:
config file → env var → CLI flag.

The `Config` struct and its resolution logic live in `src/config.rs`.

## Exercising

```
intake exercise 300
```

Records calories burned for today (decimal, e.g. `intake exercise 300.5`,
negative values rejected). On exercise days the log table shows an
`Exercise` row with a negative calorie
adjustment and a `Net` row with post-exercise calories; exercise also raises
the TDEE used for deficit calculation. `exercise_calories` is stored in the
day's log file as a bare decimal literal (`Calories`, like macros); quoted
strings are rejected on read, integers and floats are normalized. If
`show_columns` omits `calories`, the `Exercise`/`Net` rows are hidden (they
have nothing to show); the summary command likewise hides its `Exercise`
column in that case.

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
