use crate::display::{Align, Table};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Ingredient {
    pub name: String,
    pub quantity: Option<String>,
    pub protein_g: f64,
    pub fiber_g: f64,
    pub calories: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Recipe {
    pub title: String,
    pub servings: u32,
    pub ingredients: Vec<Ingredient>,
}

#[derive(Debug, Clone)]
pub struct Macros {
    pub calories: u32,
    pub protein_g: f64,
    pub fiber_g: f64,
}

impl Recipe {
    pub fn totals(&self) -> Macros {
        Macros {
            calories: self.ingredients.iter().map(|i| i.calories).sum(),
            protein_g: self.ingredients.iter().map(|i| i.protein_g).sum(),
            fiber_g: self.ingredients.iter().map(|i| i.fiber_g).sum(),
        }
    }

    pub fn per_serving(&self) -> Macros {
        let t = self.totals();
        Macros {
            calories: (t.calories as f64 / self.servings as f64).round() as u32,
            protein_g: t.protein_g / self.servings as f64,
            fiber_g: t.fiber_g / self.servings as f64,
        }
    }

    pub fn display(&self) -> String {
        let serving_label = if self.servings == 1 {
            "serving"
        } else {
            "servings"
        };

        let mut table = Table::with_align(
            &["Ingredient", "Amount", "Calories", "Protein(g)", "Fiber(g)"],
            &[
                Align::Left,
                Align::Left,
                Align::Right,
                Align::Right,
                Align::Right,
            ],
        );
        table.set_title(&format!(
            "{} ({} {})",
            self.title, self.servings, serving_label
        ));

        for ing in &self.ingredients {
            let qty = ing.quantity.as_deref().unwrap_or("-").to_string();
            table.add_row(vec![
                ing.name.clone(),
                qty,
                ing.calories.to_string(),
                format!("{:.1}g", ing.protein_g),
                format!("{:.1}g", ing.fiber_g),
            ]);
        }

        let t = self.totals();

        table.add_footer(vec![
            "Total".to_string(),
            String::new(),
            t.calories.to_string(),
            format!("{:.1}g", t.protein_g),
            format!("{:.1}g", t.fiber_g),
        ]);

        let ps = self.per_serving();
        table.add_footer(vec![
            "Per serving".to_string(),
            String::new(),
            ps.calories.to_string(),
            format!("{:.1}g", ps.protein_g),
            format!("{:.1}g", ps.fiber_g),
        ]);

        table.format()
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

pub fn list_recipe_slugs(foods_dir: &Path) -> Result<Vec<String>> {
    let mut slugs = Vec::new();
    for path in toml_files_in(foods_dir)? {
        if let Some(slug) = path.file_stem().and_then(|s| s.to_str()) {
            slugs.push(slug.to_string());
        }
    }
    Ok(slugs)
}

pub fn load_recipe(path: &Path) -> Result<Recipe> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read recipe file: {}", path.display()))?;

    let recipe: Recipe = toml::from_str(&content)
        .with_context(|| format!("failed to parse TOML in: {}", path.display()))?;

    Ok(recipe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_per_serving_with_fractions() {
        let recipe = Recipe {
            title: "Test".to_string(),
            servings: 3,
            ingredients: vec![Ingredient {
                name: "A".to_string(),
                quantity: None,
                protein_g: 10.0,
                fiber_g: 5.0,
                calories: 100,
            }],
        };
        let ps = recipe.per_serving();
        assert_eq!(ps.calories, 33);
        assert!((ps.protein_g - 3.333).abs() < 0.001);
        assert!((ps.fiber_g - 1.667).abs() < 0.001);
    }

    #[test]
    fn test_per_serving_exact_division() {
        let recipe = Recipe {
            title: "Test".to_string(),
            servings: 2,
            ingredients: vec![Ingredient {
                name: "A".to_string(),
                quantity: None,
                protein_g: 20.0,
                fiber_g: 6.0,
                calories: 100,
            }],
        };
        let ps = recipe.per_serving();
        assert_eq!(ps.calories, 50);
        assert_eq!(ps.protein_g, 10.0);
        assert_eq!(ps.fiber_g, 3.0);
    }

    #[test]
    fn test_per_serving_fractional_input() {
        let recipe = Recipe {
            title: "Fiber Test".to_string(),
            servings: 1,
            ingredients: vec![Ingredient {
                name: "Psyllium".to_string(),
                quantity: None,
                protein_g: 0.0,
                fiber_g: 0.3,
                calories: 5,
            }],
        };
        let ps = recipe.per_serving();
        assert_eq!(ps.calories, 5);
        assert_eq!(ps.fiber_g, 0.3);
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
        let recipe = Recipe {
            title: "Oatmeal".to_string(),
            servings: 2,
            ingredients: vec![
                Ingredient {
                    name: "Oats".to_string(),
                    quantity: Some("100g".to_string()),
                    protein_g: 10.0,
                    fiber_g: 5.0,
                    calories: 200,
                },
                Ingredient {
                    name: "Milk".to_string(),
                    quantity: Some("200ml".to_string()),
                    protein_g: 8.0,
                    fiber_g: 0.0,
                    calories: 120,
                },
            ],
        };

        let md = recipe.display();
        assert!(md.starts_with("\u{1b}[1;36mOatmeal (2 servings)\u{1b}[0m\n"));
        assert!(md.contains("  Oats        100g         200       10.0g"));
        assert!(md.contains("  Milk        200ml        120        8.0g"));
        assert!(md.contains("----------- ------  --------  ----------  --------"));
        assert!(md.contains("\u{1b}[1;35mTotal                    320       18.0g"));
        assert!(md.contains("\u{1b}[1;34mPer serving              160        9.0g"));
    }

    #[test]
    fn test_display_single_serving() {
        let recipe = Recipe {
            title: "Coffee".to_string(),
            servings: 1,
            ingredients: vec![Ingredient {
                name: "Cold Brew".to_string(),
                quantity: None,
                protein_g: 0.0,
                fiber_g: 0.0,
                calories: 0,
            }],
        };

        let md = recipe.display();
        assert!(md.starts_with("\u{1b}[1;36mCoffee (1 serving)\u{1b}[0m\n"));
        assert!(md.contains("  Cold Brew"));
    }

    #[test]
    fn test_display_no_quantity() {
        let recipe = Recipe {
            title: "Test".to_string(),
            servings: 1,
            ingredients: vec![Ingredient {
                name: "Secret Spice".to_string(),
                quantity: None,
                protein_g: 0.5,
                fiber_g: 0.1,
                calories: 5,
            }],
        };

        let md = recipe.display();
        assert!(md.contains("  Secret Spice"));
        assert!(md.contains("  0.5g"));
        assert!(md.contains("  0.1g"));
    }
}

pub fn find_all_recipes(foods_dir: &Path) -> Result<Vec<(PathBuf, Recipe)>> {
    let mut recipes = Vec::new();
    for path in toml_files_in(foods_dir)? {
        match load_recipe(&path) {
            Ok(recipe) => recipes.push((path, recipe)),
            Err(e) => eprintln!("Warning: skipped {}: {}", path.display(), e),
        }
    }
    Ok(recipes)
}
