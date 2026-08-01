use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
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
    #[serde(skip)]
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct PerServing {
    pub calories: u32,
    pub protein_g: f64,
    pub fiber_g: f64,
}

impl Recipe {
    pub fn per_serving(&self) -> PerServing {
        let total_cal: u32 = self.ingredients.iter().map(|i| i.calories).sum();
        let total_protein: f64 = self.ingredients.iter().map(|i| i.protein_g).sum();
        let total_fiber: f64 = self.ingredients.iter().map(|i| i.fiber_g).sum();

        PerServing {
            calories: (total_cal as f64 / self.servings as f64).round() as u32,
            protein_g: total_protein / self.servings as f64,
            fiber_g: total_fiber / self.servings as f64,
        }
    }

    pub fn hash(&self) -> &str {
        &self.content_hash
    }
}

pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)[..8].to_string()
}

pub fn load_recipe(path: &Path) -> Result<Recipe> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read recipe file: {}", path.display()))?;

    let mut recipe: Recipe = toml::from_str(&content)
        .with_context(|| format!("failed to parse TOML in: {}", path.display()))?;

    recipe.content_hash = hash_content(&content);
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
            content_hash: String::new(),
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
            content_hash: String::new(),
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
            content_hash: String::new(),
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
        assert_eq!(
            slug_from_path(Path::new("foo")),
            Some("foo".to_string())
        );
    }
}

pub fn find_all_recipes(foods_dir: &Path) -> Result<Vec<(PathBuf, Recipe)>> {
    let mut recipes = Vec::new();
    let entries = fs::read_dir(foods_dir).context("failed to read foods directory")?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "toml") {
            match load_recipe(&path) {
                Ok(recipe) => recipes.push((path, recipe)),
                Err(e) => eprintln!("Warning: skipped {}: {}", path.display(), e),
            }
        }
    }

    Ok(recipes)
}
