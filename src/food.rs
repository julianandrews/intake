use crate::amount::{calories_sum, grams_sum, Calories, Grams};
use crate::config::Column;
use crate::display::{
    food_cell, Align, ColumnValue, Table, ANSI_BOLD_YELLOW, ANSI_DIM, ANSI_RESET,
};
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

crate::display::impl_column_value!(
    Ingredient, calories, protein_g, fiber_g, fat_g, carbs_g, alcohol_g
);

#[derive(Debug, Deserialize, Clone)]
pub struct Food {
    pub title: String,
    pub servings: NonZeroU32,
    pub ingredients: Vec<Ingredient>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone)]
pub struct Macros {
    pub calories: Calories,
    pub protein_g: Grams,
    pub fiber_g: Grams,
    pub fat_g: Grams,
    pub carbs_g: Grams,
    pub alcohol_g: Grams,
}

crate::display::impl_column_value!(Macros, calories, protein_g, fiber_g, fat_g, carbs_g, alcohol_g);

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

    pub fn display(&self, columns: &[Column]) -> Result<String> {
        let serving_label = if self.servings.get() == 1 {
            "serving"
        } else {
            "servings"
        };

        let mut headers: Vec<&str> = vec!["Ingredient", "Amount"];
        let mut aligns = vec![Align::Left, Align::Left];
        for column in columns {
            headers.push(column.label());
            aligns.push(Align::Right);
        }

        let mut table = Table::with_align(&headers, &aligns);
        table.set_title(&format!(
            "{} ({} {})",
            self.title, self.servings, serving_label
        ));

        for ing in &self.ingredients {
            let qty = ing.quantity.as_deref().unwrap_or("-").to_string();
            let mut cells = vec![ing.name.clone(), qty];
            for column in columns {
                cells.push(food_cell(*column, ing.column_value(*column)));
            }
            table.add_row(cells);
        }

        let t = self.totals()?;

        let mut total = vec!["Total".to_string(), String::new()];
        for column in columns {
            total.push(food_cell(*column, t.column_value(*column)));
        }
        table.add_footer(total);

        let ps = self.per_serving()?;
        let mut per_serving = vec!["Per serving".to_string(), String::new()];
        for column in columns {
            per_serving.push(food_cell(*column, ps.column_value(*column)));
        }
        table.add_footer(per_serving);

        let mut out = table.format();
        if !self.notes.trim().is_empty() {
            out.push_str(&format!(
                "\n{ANSI_BOLD_YELLOW}Notes:{ANSI_RESET}\n{ANSI_DIM}{}{ANSI_RESET}\n",
                self.notes
            ));
        }

        Ok(out)
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
    use crate::config::DEFAULT_COLUMNS;
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

    #[test]
    fn test_display_basic() {
        let food = Food {
            title: "Oatmeal".to_string(),
            servings: NonZeroU32::new(2).unwrap(),
            notes: String::new(),
            ingredients: vec![
                Ingredient {
                    name: "Oats".to_string(),
                    quantity: Some("100g".to_string()),
                    protein_g: grams("10.0"),
                    fiber_g: grams("5.0"),
                    calories: Calories::from_u32(200),
                    fat_g: grams("4.0"),
                    carbs_g: grams("30.0"),
                    alcohol_g: grams("0.0"),
                },
                Ingredient {
                    name: "Milk".to_string(),
                    quantity: Some("200ml".to_string()),
                    protein_g: grams("8.0"),
                    fiber_g: grams("0.0"),
                    calories: Calories::from_u32(120),
                    fat_g: grams("6.0"),
                    carbs_g: grams("9.0"),
                    alcohol_g: grams("0.0"),
                },
            ],
        };

        let md = food.display(DEFAULT_COLUMNS).unwrap();
        assert!(md.starts_with("\u{1b}[1;36mOatmeal (2 servings)\u{1b}[0m\n"));
        assert!(
            md.contains("  Oats        100g         200     30.0g    4.0g       10.0g      5.0g")
        );
        assert!(
            md.contains("  Milk        200ml        120      9.0g    6.0g        8.0g      0.0g")
        );
        assert!(md.contains("----------- ------  --------  --------  ------  ----------  --------"));
        assert!(md.contains(
            "\u{1b}[1;35mTotal                    320     39.0g   10.0g       18.0g      5.0g"
        ));
        assert!(md.contains(
            "\u{1b}[1;34mPer serving              160     19.5g    5.0g        9.0g      2.5g"
        ));
    }

    #[test]
    fn test_display_single_serving() {
        let food = Food {
            title: "Coffee".to_string(),
            servings: NonZeroU32::new(1).unwrap(),
            notes: String::new(),
            ingredients: vec![Ingredient {
                name: "Cold Brew".to_string(),
                quantity: None,
                protein_g: grams("0.0"),
                fiber_g: grams("0.0"),
                calories: Calories::from_u32(0),
                fat_g: grams("0.0"),
                carbs_g: grams("0.0"),
                alcohol_g: grams("0.0"),
            }],
        };

        let md = food.display(DEFAULT_COLUMNS).unwrap();
        assert!(md.starts_with("\u{1b}[1;36mCoffee (1 serving)\u{1b}[0m\n"));
        assert!(md.contains("  Cold Brew"));
    }

    #[test]
    fn test_display_no_quantity() {
        let food = Food {
            title: "Test".to_string(),
            servings: NonZeroU32::new(1).unwrap(),
            notes: String::new(),
            ingredients: vec![Ingredient {
                name: "Secret Spice".to_string(),
                quantity: None,
                protein_g: grams("0.5"),
                fiber_g: grams("0.1"),
                calories: Calories::from_u32(5),
                fat_g: grams("0.0"),
                carbs_g: grams("0.0"),
                alcohol_g: grams("0.0"),
            }],
        };

        let md = food.display(DEFAULT_COLUMNS).unwrap();
        assert!(md.contains("  Secret Spice"));
        assert!(md.contains("  0.5g"));
        assert!(md.contains("  0.1g"));
    }

    #[test]
    fn test_display_column_subset() {
        let food = food_with_ingredient(1, ingredient("10.0", "5.0", 100, "4.0", "30.0", "0.0"));
        let md = food.display(&[Column::Calories, Column::Fat]).unwrap();
        assert!(md.contains("Calories"));
        assert!(md.contains("Fat(g)"));
        assert!(md.contains("100"));
        assert!(md.contains("4.0g"));
        assert!(!md.contains("Carbs(g)"));
        assert!(!md.contains("Protein(g)"));
    }

    fn test_food(notes: &str) -> Food {
        let mut food =
            food_with_ingredient(1, ingredient("10.0", "5.0", 100, "4.0", "30.0", "0.0"));
        food.title = "Test".to_string();
        food.notes = notes.to_string();
        food
    }

    #[test]
    fn test_display_shows_notes_when_present() {
        let md = test_food("Best eaten warm with salt.")
            .display(DEFAULT_COLUMNS)
            .unwrap();
        assert!(md.contains("Notes:"));
        assert!(md.contains("Best eaten warm with salt."));
    }

    #[test]
    fn test_display_hides_notes_when_empty() {
        let md = test_food("").display(DEFAULT_COLUMNS).unwrap();
        assert!(!md.contains("Notes:"));
    }

    #[test]
    fn test_display_hides_notes_when_whitespace() {
        let md = test_food("   ").display(DEFAULT_COLUMNS).unwrap();
        assert!(!md.contains("Notes:"));
    }
}
