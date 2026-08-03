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

#[derive(Debug, Clone)]
pub enum Align {
    Left,
    Right,
}

#[derive(Debug)]
pub struct Table {
    headers: Vec<String>,
    align: Vec<Align>,
    rows: Vec<Vec<String>>,
    title: Option<String>,
    footers: Vec<(String, Vec<String>)>,
}

pub fn visible_width(s: &str) -> usize {
    // Strip ANSI escape sequences for column-width calculation
    let mut len = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            // consume until 'm'
            for n in chars.by_ref() {
                if n == 'm' {
                    break;
                }
            }
        } else {
            len += 1;
        }
    }
    len
}

fn format_cells(cells: &[String], widths: &[usize], align: &[Align]) -> String {
    let mut line = String::new();
    for (i, (cell, width)) in cells.iter().zip(widths).enumerate() {
        let vis = visible_width(cell);
        let pad = width.saturating_sub(vis);
        match align[i] {
            Align::Left => {
                write!(line, "{}", cell).unwrap();
                for _ in 0..pad {
                    line.push(' ');
                }
            }
            Align::Right => {
                for _ in 0..pad {
                    line.push(' ');
                }
                write!(line, "{}", cell).unwrap();
            }
        }
        if i == 0 {
            line.push(' ');
        } else if i < cells.len() - 1 {
            line.push_str("  ");
        }
    }
    line
}

impl Table {
    pub fn new(headers: &[&str]) -> Self {
        let n = headers.len();
        let mut align = vec![Align::Right; n];
        if n > 0 {
            align[0] = Align::Left;
        }
        Table {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            align,
            rows: Vec::new(),
            title: None,
            footers: Vec::new(),
        }
    }

    pub fn with_align(headers: &[&str], align: &[Align]) -> Self {
        Table {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            align: align.to_vec(),
            rows: Vec::new(),
            title: None,
            footers: Vec::new(),
        }
    }

    pub fn add_row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = Some(title.to_string());
    }

    pub fn add_footer(&mut self, label: &str, cells: Vec<String>) {
        assert_eq!(
            cells.len(),
            self.headers.len() - 1,
            "add_footer expects {} cells (one per data column), got {}",
            self.headers.len() - 1,
            cells.len()
        );
        self.footers.push((label.to_string(), cells));
    }

    fn col_widths(&self) -> Vec<usize> {
        let mut widths: Vec<usize> = self.headers.iter().map(|h| visible_width(h)).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(visible_width(cell));
            }
        }
        for (label, cells) in &self.footers {
            widths[0] = widths[0].max(visible_width(label));
            for (i, cell) in cells.iter().enumerate() {
                widths[i] = widths[i].max(visible_width(cell));
            }
        }
        widths
    }

    pub fn format(&self) -> String {
        let widths = self.col_widths();
        let n = widths.len();
        let sep_total = if n <= 1 { 0 } else { 1 + 2 * (n - 2) };
        let sep_width = 2 + widths.iter().sum::<usize>() + sep_total;

        let mut out = String::new();

        if let Some(title) = &self.title {
            writeln!(out, "{ANSI_BOLD_CYAN}{title}{ANSI_RESET}").unwrap();
        }

        writeln!(out, "{ANSI_CYAN}{}{ANSI_RESET}", "-".repeat(sep_width)).unwrap();

        writeln!(
            out,
            "  {ANSI_BOLD_YELLOW}{}{ANSI_RESET}",
            format_cells(&self.headers, &widths, &self.align)
        )
        .unwrap();

        let dash_cells: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
        writeln!(
            out,
            "  {ANSI_CYAN}{}{ANSI_RESET}",
            format_cells(&dash_cells, &widths, &self.align)
        )
        .unwrap();

        for row in &self.rows {
            writeln!(out, "  {}", format_cells(row, &widths, &self.align)).unwrap();
        }

        writeln!(out, "{ANSI_CYAN}{}{ANSI_RESET}", "-".repeat(sep_width)).unwrap();

        for (i, (label, cells)) in self.footers.iter().enumerate() {
            let mut full_row = vec![label.clone()];
            full_row.extend(cells.iter().cloned());

            let ansi = if i == 0 {
                ANSI_BOLD_GREEN
            } else {
                ANSI_BOLD_CYAN
            };
            writeln!(
                out,
                "  {ansi}{}{ANSI_RESET}",
                format_cells(&full_row, &widths, &self.align)
            )
            .unwrap();
        }

        out
    }
}

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

        let total_cal: u32 = self.ingredients.iter().map(|i| i.calories).sum();
        let total_protein: f64 = self.ingredients.iter().map(|i| i.protein_g).sum();
        let total_fiber: f64 = self.ingredients.iter().map(|i| i.fiber_g).sum();

        table.add_footer(
            "Total",
            vec![
                String::new(),
                total_cal.to_string(),
                format!("{:.1}g", total_protein),
                format!("{:.1}g", total_fiber),
            ],
        );

        let ps = self.per_serving();
        table.add_footer(
            "Per serving",
            vec![
                String::new(),
                ps.calories.to_string(),
                format!("{:.1}g", ps.protein_g),
                format!("{:.1}g", ps.fiber_g),
            ],
        );

        table.format()
    }
}

fn toml_files_in(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let entries = fs::read_dir(dir).context("failed to read foods directory")?;
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
