# intake

A CLI diet tracker written in Rust. Usage lives in `README.md` and `intake --help`.

## Model

Foods are templates for log entries: `add` loads a food file, computes its
per-serving macros, and copies them (plus the food's title) into a new
standalone log entry. Entries are flat — `title`, `servings`, and the six
macros — with no reference back to the food file, so editing a food never
changes existing log entries and logs render without reading the foods
directory.

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

1. `cargo test` — all tests must pass
2. `cargo clippy -- -D warnings` — no clippy warnings (deny all)
3. `cargo fmt --check` — formatting must match `rustfmt`
4. `cargo build` — clean build with no warnings
