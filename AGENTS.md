# intake

A CLI diet tracker written in Rust. Usage lives in `README.md` and `intake --help`.

## Model

Foods are templates for log entries: `log` loads a food file, computes its
per-serving macros, and copies them (plus the food's title) into a new
standalone log entry. Entries are flat — `title`, `servings`, and the six
macros — with no reference back to the food file, so editing a food never
changes existing log entries and logs render without reading the foods
directory.

## Workspace

`intake` is a Cargo workspace. The binary lives at the root; `intake-ai` is
a library under `crates/` that implements the generic AI pipeline
(settings, LLM backend, agent loop with tools, resolve loop with
confirmation). It knows nothing about intake, food, nutrition, or TOML;
intake supplies the parse closures, prompt templates, `food_lookup` and
`usda_search` tools, and confirmation UX.

AI support is the `ai` Cargo feature (on by default), gated module-level:
`#[cfg(feature = "ai")] mod ai;` in main.rs, the `Ai` variant on
`Commands` in cli.rs, its match arm in `commands/mod.rs`, and the `[ai]`
field in `Config` — four attributes in total, none inside `src/ai/`. All
ai-only code lives inside `src/ai/` (clap tree, config wrapper, confirmer,
checked day writes, food catalog, USDA search tool); shared files carry no
per-item cfg.

This is a non-virtual workspace (the root is a package), so bare `cargo
test` / `clippy` / `build` from the root operate on the `intake` package
only. The quality gates below use `--workspace` so `intake-ai`'s own tests
and lints are covered too.

## Conventions

- No floating point anywhere: all amounts are exact decimals (0.001
  precision) via the types in `amount.rs`. Arithmetic is checked — overflow
  fails loudly with an error, never panics or wraps.
- Rounding happens only at the display boundary (0.1 g, whole kcal).
- Strict parsing: bare TOML numbers only, quoted strings rejected; missing or
  invalid fields error instead of defaulting.
- Unit tests are inline (`#[cfg(test)]` per module); end-to-end tests live in
  `tests/cli.rs` with fixtures in `tests/fixtures/`.

## Code Quality

Run all four from the project root before committing or finishing a task:

1. `cargo test --workspace` — all tests must pass
2. `cargo clippy --workspace -- -D warnings` — no clippy warnings (deny all)
3. `cargo fmt --check` — formatting must match `rustfmt`
4. `cargo build --workspace` — clean build with no warnings

Plus, for the no-AI configuration (scoped to the `intake` package, so the
`intake-ai` deps stay out of the no-AI build):

5. `cargo test -p intake --no-default-features`
6. `cargo build -p intake --no-default-features`

All are non-destructive: they only read source and write build artifacts
under `target/`, never touching tracked files — safe to run in plan mode to
verify a plan before finalizing it.
