use crate::amount::{calories_sum, grams_sum, Calories, Grams, Macros};
use anyhow::{anyhow, Context, Result};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
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

#[derive(Debug, Deserialize, Clone)]
pub struct Food {
    pub title: String,
    pub servings: NonZeroU32,
    pub ingredients: Vec<Ingredient>,
    #[serde(default)]
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

fn toml_files_in(dir: &Path) -> Result<Vec<PathBuf>> {
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
    Ok(files)
}

pub fn list_food_slugs(foods_dir: &Path) -> Result<Vec<String>> {
    let mut slugs = Vec::new();
    for path in toml_files_in(foods_dir)? {
        if let Some(slug) = path.file_stem().and_then(|s| s.to_str()) {
            slugs.push(slug.to_string());
        }
    }
    Ok(slugs)
}

pub fn load_food(path: &Path) -> Result<Food> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read food file: {}", path.display()))?;

    let food: Food = toml::from_str(&content)
        .with_context(|| format!("failed to parse TOML in: {}", path.display()))?;

    Ok(food)
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
            calories: Calories::from_u32(calories),
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
        assert_eq!(food.totals().unwrap().calories, Calories::from_u32(100));
    }

    #[test]
    fn test_per_serving_exact_division() {
        let food = food_with_ingredient(2, ingredient("20.0", "6.0", 100, "4.0", "30.0", "2.0"));
        let ps = food.per_serving().unwrap();
        assert_eq!(ps.calories, Calories::from_u32(50));
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
        assert_eq!(ps.calories, Calories::from_u32(5));
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

    fn slug_from_path(path: &Path) -> Option<String> {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    }

    #[test]
    fn test_slug_from_path() {
        assert_eq!(
            slug_from_path(Path::new("foods/coffee.toml")),
            Some("coffee".to_string())
        );
        assert_eq!(
            slug_from_path(Path::new("coffee.toml")),
            Some("coffee".to_string())
        );
        assert_eq!(slug_from_path(Path::new("")), None);
    }

    #[test]
    fn test_slug_from_path_no_extension() {
        assert_eq!(slug_from_path(Path::new("foo")), Some("foo".to_string()));
    }
}
