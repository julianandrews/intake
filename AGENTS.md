# Adding a Recipe

Create a `.toml` file in `foods/`. The filename (minus `.toml`) becomes the recipe slug.

The file must match the `Recipe` and `Ingredient` structs in `diet-tracker/src/recipe.rs`:

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
