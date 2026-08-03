# intake

This project is a CLI diet tracker. Use the `intake` binary for all operations —
run `intake --help` to see available subcommands and flags.


                                                              ## Adding a Recipe

Create a `.toml` file in `foods/`. The filename (minus `.toml`) becomes the recipe slug.

The file must match the `Recipe` and `Ingredient` structs in `src/recipe.rs`:

- `title` — display name
- `servings` — how many servings the full recipe makes
- `[[ingredients]]` — one table per ingredient, each with:
  - `name` (string, required)
  - `quantity` (string, optional — e.g. `"400g"`, `"1 tbsp"`)
  - `protein_g` (float or int)
  - `fiber_g` (float or int)
  - `calories` (int)

See `foods/cheesy-popcorn.toml` for a simple example, or `foods/turkey-chili.toml` for a multi-ingredient one.

The `content_hash` field is computed automatically at load time — do not include it.

## Adding an Adhoc Log Entry

Use the `adhoc` CLI subcommand for one-off foods without a recipe file:

```
intake adhoc --calories N --protein N --fiber N <name> [servings]
```

- Macros are specified inline — no recipe file needed
- The entry is appended to today's log in `log/YYYY-MM-DD.toml`
- Slug is auto-derived from the name (lowercased, spaces → hyphens)
- Check existing entries in `log/` for the exact TOML format to follow (especially adhoc entries with a `title` field)
- See existing examples in `log/2026-08-01.toml` and `log/2026-08-02.toml`
- Implementation: `src/main.rs` function `cmd_adhoc`
- When you need to look up macros for a food, use `websearch` to find reliable nutrition data (e.g. from the USDA). Prefer data per 100g and scale by the serving size.

## Code Quality

Always run these checks before committing or finishing a task:

1. **`cargo test`** — all tests must pass
2. **`cargo clippy -- -D warnings`** — no clippy warnings (deny all)
3. **`cargo fmt --check`** — formatting must match `rustfmt`
4. **`cargo build`** — clean build with no warnings

Run them in the project root.
