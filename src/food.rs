use crate::amount::{calories_sum, grams_sum, Calories, Grams, Macros};
use anyhow::{anyhow, bail, Context, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Write;
use std::num::NonZeroU32;
use std::path::{Component, Path, PathBuf, MAIN_SEPARATOR};
use std::str::FromStr;

/// A food name: the filename (minus `.toml`) used to look up a food.
///
/// Validated on construction, so invalid names fail at parse time instead
/// of during a filesystem lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoodName(String);

impl FoodName {
    /// The path of the food's TOML file inside `foods_dir`.
    pub fn file_path(&self, foods_dir: &Path) -> PathBuf {
        foods_dir.join(format!("{}.toml", self.0))
    }
}

impl FromStr for FoodName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err("food name must not be empty".to_string());
        }
        if s.ends_with(MAIN_SEPARATOR) {
            return Err(format!(
                "food name '{s}' must not end with a path separator"
            ));
        }
        let mut components = Path::new(s).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(name)), None) => {
                Ok(FoodName(name.to_string_lossy().into_owned()))
            }
            _ => Err(format!("food name '{s}' is not a valid filename")),
        }
    }
}

impl fmt::Display for FoodName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ingredient {
    pub name: String,
    pub quantity: Option<String>,
    pub protein_g: Grams,
    pub fiber_g: Grams,
    pub calories: Calories,
    pub fat_g: Grams,
    pub carbs_g: Grams,
    pub alcohol_g: Grams,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Food {
    pub title: String,
    pub servings: NonZeroU32,
    pub ingredients: Vec<Ingredient>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

impl Food {
    pub fn totals(&self) -> Result<Macros> {
        Ok(Macros {
            calories: calories_sum(self.ingredients.iter().map(|i| i.calories))
                .ok_or_else(|| anyhow!("calorie total overflow"))?,
            protein_g: grams_sum(self.ingredients.iter().map(|i| i.protein_g))
                .ok_or_else(|| anyhow!("protein total overflow"))?,
            fiber_g: grams_sum(self.ingredients.iter().map(|i| i.fiber_g))
                .ok_or_else(|| anyhow!("fiber total overflow"))?,
            fat_g: grams_sum(self.ingredients.iter().map(|i| i.fat_g))
                .ok_or_else(|| anyhow!("fat total overflow"))?,
            carbs_g: grams_sum(self.ingredients.iter().map(|i| i.carbs_g))
                .ok_or_else(|| anyhow!("carbs total overflow"))?,
            alcohol_g: grams_sum(self.ingredients.iter().map(|i| i.alcohol_g))
                .ok_or_else(|| anyhow!("alcohol total overflow"))?,
        })
    }

    pub fn per_serving(&self) -> Result<Macros> {
        let t = self.totals()?;
        let servings = Decimal::from(self.servings.get());
        // Serving count is a nonzero u32, so division can neither hit zero nor
        // overflow: x / servings <= x <= Decimal::MAX.
        Ok(Macros {
            calories: t
                .calories
                .checked_div(servings)
                .expect("serving count is positive"),
            protein_g: t
                .protein_g
                .checked_div(servings)
                .expect("serving count is positive"),
            fiber_g: t
                .fiber_g
                .checked_div(servings)
                .expect("serving count is positive"),
            fat_g: t
                .fat_g
                .checked_div(servings)
                .expect("serving count is positive"),
            carbs_g: t
                .carbs_g
                .checked_div(servings)
                .expect("serving count is positive"),
            alcohol_g: t
                .alcohol_g
                .checked_div(servings)
                .expect("serving count is positive"),
        })
    }
}

pub(crate) fn toml_files_in(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let entries = fs::read_dir(dir)
        .with_context(|| format!("foods directory not found: {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

pub fn list_food_names(foods_dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for path in toml_files_in(foods_dir)? {
        if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

pub fn load_food(path: &Path) -> Result<Food> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read food file: {}", path.display()))?;

    let food: Food = toml::from_str(&content)
        .with_context(|| format!("failed to parse TOML in: {}", path.display()))?;

    Ok(food)
}

/// Overwrite a food file for `name` in `foods_dir`, atomically.
pub fn write_food(foods_dir: &Path, name: &FoodName, food: &Food) -> Result<()> {
    write_food_impl(foods_dir, name, food, true)
}

/// Create a food file for `name` in `foods_dir`, atomically.
///
/// Fails instead of overwriting if the file exists, so a concurrent `food new`
/// for the same name cannot clobber an existing file.
pub fn create_food(foods_dir: &Path, name: &FoodName, food: &Food) -> Result<()> {
    let path = name.file_path(foods_dir);
    match write_food_impl(foods_dir, name, food, false) {
        Ok(()) => Ok(()),
        Err(_) if path.exists() => {
            bail!(
                "food '{}' already exists — use `food edit {}` to modify it",
                name,
                name
            )
        }
        Err(e) => Err(e),
    }
}

fn write_food_impl(foods_dir: &Path, name: &FoodName, food: &Food, clobber: bool) -> Result<()> {
    let path = name.file_path(foods_dir);

    fs::create_dir_all(foods_dir)
        .with_context(|| format!("failed to create foods directory: {}", foods_dir.display()))?;

    let content = toml::to_string(food).context("failed to serialize food")?;
    let mut tmp = tempfile::NamedTempFile::new_in(foods_dir).with_context(|| {
        format!(
            "failed to create temporary food in: {}",
            foods_dir.display()
        )
    })?;
    tmp.write_all(content.as_bytes())
        .with_context(|| format!("failed to write temporary food: {}", path.display()))?;
    tmp.as_file()
        .sync_all()
        .with_context(|| format!("failed to sync temporary food: {}", path.display()))?;

    let persist_result = if clobber {
        tmp.persist(&path)
    } else {
        tmp.persist_noclobber(&path)
    };
    persist_result
        .map(|_| ())
        .with_context(|| format!("failed to write food: {}", path.display()))?;

    Ok(())
}

/// Delete the food file for `name` in `foods_dir`.
///
/// Log entries are flat copies of a food's values, so removing the file never
/// affects existing log entries.
pub fn remove_food(foods_dir: &Path, name: &FoodName) -> Result<()> {
    let path = name.file_path(foods_dir);
    if !path.exists() {
        bail!("food '{}' not found", name);
    }
    fs::remove_file(&path).with_context(|| format!("failed to remove food: {}", path.display()))?;
    // Sync the directory so the unlink is durable, matching the sync before
    // the atomic rename in write_food_impl and the day-log removal.
    fs::File::open(foods_dir)
        .with_context(|| format!("failed to open foods directory: {}", foods_dir.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync foods directory: {}", foods_dir.display()))?;
    Ok(())
}

pub fn find_all_foods(foods_dir: &Path) -> Result<Vec<Food>> {
    let mut foods = Vec::new();
    for path in toml_files_in(foods_dir)? {
        match load_food(&path) {
            Ok(food) => foods.push(food),
            Err(e) => eprintln!("Warning: skipped {}: {}", path.display(), e),
        }
    }
    Ok(foods)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;
    use std::str::FromStr;

    fn grams(value: &str) -> Grams {
        Grams::from_str(value).unwrap()
    }

    fn food_with_ingredient(servings: u32, ingredient: Ingredient) -> Food {
        Food {
            title: "Test".to_string(),
            servings: NonZeroU32::new(servings).unwrap(),
            notes: String::new(),
            ingredients: vec![ingredient],
        }
    }

    fn ingredient(
        protein: &str,
        fiber: &str,
        calories: u32,
        fat: &str,
        carbs: &str,
        alcohol: &str,
    ) -> Ingredient {
        Ingredient {
            name: "A".to_string(),
            quantity: None,
            protein_g: grams(protein),
            fiber_g: grams(fiber),
            calories: Calories::from_str(&calories.to_string()).unwrap(),
            fat_g: grams(fat),
            carbs_g: grams(carbs),
            alcohol_g: grams(alcohol),
        }
    }

    #[test]
    fn test_per_serving_with_fractions() {
        let food = food_with_ingredient(3, ingredient("10.0", "5.0", 100, "0.0", "0.0", "0.0"));
        let ps = food.per_serving().unwrap();
        assert_eq!(ps.calories, Calories::from_str("33.333").unwrap());
        assert_eq!(ps.protein_g, Grams::from_str("3.333").unwrap());
        assert_eq!(ps.fiber_g, Grams::from_str("1.667").unwrap());
    }

    #[test]
    fn test_per_serving_calories_keep_fractional_precision() {
        let food = food_with_ingredient(3, ingredient("0.0", "0.0", 100, "0.0", "0.0", "0.0"));
        let ps = food.per_serving().unwrap();
        assert_eq!(ps.calories, Calories::from_str("33.333").unwrap());
        assert_eq!(
            food.totals().unwrap().calories,
            Calories::from_str("100").unwrap()
        );
    }

    #[test]
    fn test_per_serving_exact_division() {
        let food = food_with_ingredient(2, ingredient("20.0", "6.0", 100, "4.0", "30.0", "2.0"));
        let ps = food.per_serving().unwrap();
        assert_eq!(ps.calories, Calories::from_str("50").unwrap());
        assert_eq!(ps.protein_g, grams("10.0"));
        assert_eq!(ps.fiber_g, grams("3.0"));
        assert_eq!(ps.fat_g, grams("2.0"));
        assert_eq!(ps.carbs_g, grams("15.0"));
        assert_eq!(ps.alcohol_g, grams("1.0"));
    }

    #[test]
    fn test_per_serving_fractional_input() {
        let food = food_with_ingredient(1, ingredient("0.0", "0.3", 5, "0.0", "0.0", "0.0"));
        let ps = food.per_serving().unwrap();
        assert_eq!(ps.calories, Calories::from_str("5").unwrap());
        assert_eq!(ps.fiber_g, grams("0.3"));
    }

    #[test]
    fn test_ingredient_missing_macros_rejected() {
        let result: Result<Food, _> =
            toml::from_str("title = \"X\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\ncalories = 10\nprotein_g = 1\nfiber_g = 0\n");
        assert!(result.is_err());
    }

    #[test]
    fn test_zero_servings_rejected() {
        let result: Result<Food, _> = toml::from_str(
            "title = \"X\"\nservings = 0\n\n[[ingredients]]\nname = \"A\"\ncalories = 10\nprotein_g = 1\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        );
        assert!(result.is_err());
    }

    fn name_from_path(path: &Path) -> Option<String> {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    }

    #[test]
    fn test_name_from_path() {
        assert_eq!(
            name_from_path(Path::new("foods/coffee.toml")),
            Some("coffee".to_string())
        );
        assert_eq!(
            name_from_path(Path::new("coffee.toml")),
            Some("coffee".to_string())
        );
        assert_eq!(name_from_path(Path::new("")), None);
    }

    #[test]
    fn test_name_from_path_no_extension() {
        assert_eq!(name_from_path(Path::new("foo")), Some("foo".to_string()));
    }

    #[test]
    fn test_food_name_parses_valid_names() {
        for s in ["coffee", "quest-bar", "spicy-potato-wedges", "x"] {
            assert_eq!(FoodName::from_str(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn test_food_name_rejects_empty_and_traversal() {
        assert!(FoodName::from_str("").is_err());
        assert!(FoodName::from_str("a/b").is_err());
        assert!(FoodName::from_str(".").is_err());
        assert!(FoodName::from_str("..").is_err());
    }

    #[test]
    fn test_food_name_rejects_trailing_separator() {
        assert!(FoodName::from_str("coffee/").is_err());
        assert!(FoodName::from_str("coffee//").is_err());
    }

    #[test]
    fn test_food_name_accepts_backslash() {
        assert!(FoodName::from_str("a\\b").is_ok());
    }

    #[test]
    fn test_food_name_file_path() {
        assert_eq!(
            FoodName::from_str("quest-bar")
                .unwrap()
                .file_path(Path::new("foods")),
            PathBuf::from("foods/quest-bar.toml")
        );
    }

    #[test]
    fn test_food_serialize_roundtrip() {
        let food = food_with_ingredient(2, ingredient("10.0", "5.0", 100, "4.0", "30.0", "2.0"));
        let serialized = toml::to_string(&food).unwrap();
        let deserialized: Food = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.title, food.title);
        assert_eq!(deserialized.servings, food.servings);
        assert_eq!(
            deserialized.ingredients[0].calories,
            food.ingredients[0].calories
        );
        assert_eq!(
            deserialized.ingredients[0].protein_g,
            food.ingredients[0].protein_g
        );
    }

    #[test]
    fn test_food_serialize_skips_empty_notes() {
        let food = food_with_ingredient(1, ingredient("0.0", "0.0", 5, "0.0", "0.0", "0.0"));
        let serialized = toml::to_string(&food).unwrap();
        assert!(!serialized.contains("notes"));
    }

    #[test]
    fn test_food_serialize_keeps_notes_when_present() {
        let mut food = food_with_ingredient(1, ingredient("0.0", "0.0", 5, "0.0", "0.0", "0.0"));
        food.notes = "Best eaten warm".to_string();
        let serialized = toml::to_string(&food).unwrap();
        let deserialized: Food = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.notes, "Best eaten warm");
    }

    #[test]
    fn test_food_unknown_field_rejected() {
        let result: Result<Food, _> = toml::from_str(
            "title = \"X\"\nservings = 1\nbogus = 1\n\n[[ingredients]]\nname = \"A\"\ncalories = 10\nprotein_g = 1\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_ingredient_unknown_field_rejected() {
        let result: Result<Food, _> = toml::from_str(
            "title = \"X\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\nbogus = 1\ncalories = 10\nprotein_g = 1\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_write_food_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let food = food_with_ingredient(2, ingredient("10.0", "5.0", 100, "4.0", "30.0", "2.0"));
        let name = FoodName::from_str("test-food").unwrap();
        write_food(dir.path(), &name, &food).unwrap();
        let loaded = load_food(&name.file_path(dir.path())).unwrap();
        assert_eq!(loaded.title, food.title);
        assert_eq!(loaded.ingredients.len(), 1);
        assert_eq!(loaded.ingredients[0].calories, food.ingredients[0].calories);
    }

    #[test]
    fn test_remove_food_deletes_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let food = food_with_ingredient(1, ingredient("0.0", "0.0", 5, "0.0", "0.0", "0.0"));
        let name = FoodName::from_str("test-food").unwrap();
        write_food(dir.path(), &name, &food).unwrap();
        assert!(name.file_path(dir.path()).exists());

        remove_food(dir.path(), &name).unwrap();
        assert!(!name.file_path(dir.path()).exists());
    }

    #[test]
    fn test_remove_food_not_found_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = FoodName::from_str("ghost-food").unwrap();
        let err = remove_food(dir.path(), &name).unwrap_err();
        assert!(err.to_string().contains("ghost-food"));
        assert!(err.to_string().contains("not found"));
    }
}
