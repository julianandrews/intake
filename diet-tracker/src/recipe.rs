use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";

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

    pub fn display(&self) -> String {
        let serving_label = if self.servings == 1 { "serving" } else { "servings" };
        let mut out = format!("{ANSI_BOLD_CYAN}{} ({} {}){ANSI_RESET}\n", self.title, self.servings, serving_label);

        let c_ing = self.ingredients.iter()
            .map(|i| i.name.len()).chain([10, 5, 11]).max().unwrap();
        let c_qty = self.ingredients.iter()
            .filter_map(|i| i.quantity.as_deref().map(|q| q.len()))
            .chain([6]).max().unwrap();
        let c_cal = self.ingredients.iter()
            .map(|i| i.calories.to_string().len()).chain([3, 8]).max().unwrap();
        let c_prot = self.ingredients.iter()
            .map(|i| format!("{:.1}g", i.protein_g).len()).chain([7, 10]).max().unwrap();
        let c_fib = self.ingredients.iter()
            .map(|i| format!("{:.1}g", i.fiber_g).len()).chain([5, 8]).max().unwrap();

        let sep_width = 2 + c_ing + 1 + c_qty + 2 + c_cal + 2 + c_prot + 2 + c_fib;

        writeln!(out, "{ANSI_CYAN}{}{ANSI_RESET}", "-".repeat(sep_width)).unwrap();

        writeln!(
            out,
            "  {ANSI_BOLD_YELLOW}{:<i$} {:<q$}  {:>c$}  {:>p$}  {:>f$}{ANSI_RESET}",
            "Ingredient", "Amount", "Calories", "Protein(g)", "Fiber(g)",
            i = c_ing, q = c_qty, c = c_cal, p = c_prot, f = c_fib
        ).unwrap();

        writeln!(
            out,
            "  {ANSI_CYAN}{:<i$} {:<q$}  {:>c$}  {:>p$}  {:>f$}{ANSI_RESET}",
            "-".repeat(c_ing), "-".repeat(c_qty), "-".repeat(c_cal), "-".repeat(c_prot), "-".repeat(c_fib),
            i = c_ing, q = c_qty, c = c_cal, p = c_prot, f = c_fib
        ).unwrap();

        for ing in &self.ingredients {
            let qty = ing.quantity.as_deref().unwrap_or("-");
            let prot = format!("{:.1}g", ing.protein_g);
            let fib = format!("{:.1}g", ing.fiber_g);
            writeln!(
                out,
                "  {:<i$} {:<q$}  {:>c$}  {:>p$}  {:>f$}",
                ing.name, qty, ing.calories, prot, fib,
                i = c_ing, q = c_qty, c = c_cal, p = c_prot, f = c_fib
            ).unwrap();
        }

        writeln!(out, "{ANSI_CYAN}{}{ANSI_RESET}", "-".repeat(sep_width)).unwrap();

        let total_cal: u32 = self.ingredients.iter().map(|i| i.calories).sum();
        let total_protein: f64 = self.ingredients.iter().map(|i| i.protein_g).sum();
        let total_fiber: f64 = self.ingredients.iter().map(|i| i.fiber_g).sum();
        let total_prot_str = format!("{:.1}g", total_protein);
        let total_fib_str = format!("{:.1}g", total_fiber);

        writeln!(
            out,
            "  {ANSI_BOLD_GREEN}{:<i$} {:<q$}  {:>c$}  {:>p$}  {:>f$}{ANSI_RESET}",
            "Total", "", total_cal, total_prot_str, total_fib_str,
            i = c_ing, q = c_qty, c = c_cal, p = c_prot, f = c_fib
        ).unwrap();

        let ps = self.per_serving();
        let ps_prot_str = format!("{:.1}g", ps.protein_g);
        let ps_fib_str = format!("{:.1}g", ps.fiber_g);

        writeln!(
            out,
            "  {ANSI_BOLD_CYAN}{:<i$} {:<q$}  {:>c$}  {:>p$}  {:>f$}{ANSI_RESET}",
            "Per serving", "", ps.calories, ps_prot_str, ps_fib_str,
            i = c_ing, q = c_qty, c = c_cal, p = c_prot, f = c_fib
        ).unwrap();

        out
    }
}

fn toml_files_in(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let entries = fs::read_dir(dir).context("failed to read foods directory")?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "toml") {
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
            content_hash: String::new(),
        };

        let md = recipe.display();
        assert!(md.starts_with("\u{1b}[1;36mOatmeal (2 servings)\u{1b}[0m\n"));
        assert!(md.contains("  Oats        100g         200       10.0g"));
        assert!(md.contains("  Milk        200ml        120        8.0g"));
        assert!(md.contains("----------- ------  --------  ----------  --------"));
        assert!(md.contains("\u{1b}[1;32mTotal                    320       18.0g"));
        assert!(md.contains("\u{1b}[1;36mPer serving              160        9.0g"));
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
            content_hash: String::new(),
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
            content_hash: String::new(),
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
