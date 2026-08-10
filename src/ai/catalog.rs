use crate::food;
use anyhow::Result;
use std::path::Path;

/// Every food in `foods_dir` paired with its name (the filename without
/// `.toml`), sorted by name. Foods that fail to parse are skipped with a
/// stderr warning, like [`food::find_all_foods`].
pub(crate) fn find_all_foods_with_names(foods_dir: &Path) -> Result<Vec<(String, food::Food)>> {
    let mut foods = Vec::new();
    for path in food::toml_files_in(foods_dir)? {
        let Some(name) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        match food::load_food(&path) {
            Ok(food) => foods.push((name, food)),
            Err(e) => eprintln!("Warning: skipped {}: {}", path.display(), e),
        }
    }
    Ok(foods)
}
