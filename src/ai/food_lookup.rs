use crate::food;
use intake_ai::tools::Tool;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

use super::catalog;

fn eprintln_warn(msg: &str) {
    eprintln!("{msg}");
}

const MAX_PER_QUERY: usize = 5;
const TOTAL_CAP: usize = 2000;

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub struct FoodLookup<'a> {
    foods_dir: &'a Path,
    warn: Option<&'a dyn Fn(&str)>,
}

impl<'a> FoodLookup<'a> {
    pub fn new(foods_dir: &'a Path) -> FoodLookup<'a> {
        FoodLookup {
            foods_dir,
            warn: None,
        }
    }

    /// Routes catalog warnings through `warn` instead of the default stderr
    /// print, so a running status line can print them without the spinner
    /// garbling them.
    pub fn with_warn(mut self, warn: &'a dyn Fn(&str)) -> FoodLookup<'a> {
        self.warn = Some(warn);
        self
    }

    fn catalog(&self) -> Result<Vec<(String, food::Food)>, String> {
        let warn: &dyn Fn(&str) = self.warn.unwrap_or(&eprintln_warn);
        catalog::find_all_foods_with_names(self.foods_dir, warn).map_err(|e| e.to_string())
    }

    fn query_block(catalog: &[(String, food::Food)], query: &str) -> Result<String, String> {
        let q = normalize(query);
        let mut out = format!("query: {query}\n");
        if q.is_empty() {
            out.push_str("  no matches\n");
            return Ok(out);
        }
        let mut exact: Vec<(&String, &food::Food)> = Vec::new();
        let mut partial: Vec<(&String, &food::Food)> = Vec::new();
        for (name, f) in catalog {
            let norm_name = normalize(name);
            let norm_title = normalize(&f.title);
            if norm_name == q || norm_title == q {
                exact.push((name, f));
            } else if norm_name.contains(&q)
                || norm_title.contains(&q)
                || q.contains(&norm_name)
                || q.contains(&norm_title)
            {
                partial.push((name, f));
            }
        }
        exact.sort_by(|a, b| a.0.cmp(b.0));
        partial.sort_by(|a, b| a.0.cmp(b.0));
        exact.extend(partial);

        let mut seen: HashSet<String> = HashSet::new();
        let mut count = 0usize;
        for (name, f) in exact {
            if !seen.insert(name.clone()) {
                continue;
            }
            if count >= MAX_PER_QUERY {
                break;
            }
            let ps = f.per_serving().map_err(|e| format!("food '{name}': {e}"))?;
            let line = format!(
                "{name} | {} | {} cal/serv, {} protein_g, {} fiber_g, {} fat_g, {} carbs_g, {} alcohol_g",
                f.title,
                ps.calories,
                ps.protein_g,
                ps.fiber_g,
                ps.fat_g,
                ps.carbs_g,
                ps.alcohol_g
            );
            if out.chars().count() + line.chars().count() > TOTAL_CAP {
                out.push_str("  …\n");
                break;
            }
            count += 1;
            out.push_str(&format!("  {count}. {line}\n"));
        }
        if count == 0 {
            out.push_str("  no matches\n");
        }
        Ok(out)
    }
}

impl Tool for FoodLookup<'_> {
    fn name(&self) -> &str {
        "food_lookup"
    }

    fn description(&self) -> &str {
        "Look up the user's own foods by name or title. Accepts a batch of queries and returns up to five catalog lines per query with per-serving macros. Before emitting any add-adhoc op, include the intended title in a batched food_lookup call."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "queries": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "description": "One or more food titles to look up."
                }
            },
            "required": ["queries"]
        })
    }

    fn execute(&self, params: &Value) -> Result<String, String> {
        let queries = params["queries"]
            .as_array()
            .ok_or_else(|| "food_lookup: missing 'queries' array".to_string())?
            .iter()
            .filter_map(|q| q.as_str())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if queries.is_empty() {
            return Err("food_lookup: 'queries' must contain at least one string".to_string());
        }
        let catalog = self.catalog()?;
        let mut out = String::new();
        for (i, query) in queries.iter().enumerate() {
            let block = Self::query_block(&catalog, query)?;
            if i > 0 {
                out.push('\n');
            }
            if out.chars().count() + block.chars().count() > TOTAL_CAP {
                out.push_str("… (output truncated)\n");
                break;
            }
            out.push_str(&block);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foods_dir() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let foods = [
            (
                "coffee",
                "title = \"Coffee\"\nservings = 1\n\n[[ingredients]]\nname = \"Beans\"\ncalories = 12\nprotein_g = 1\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
            ),
            (
                "oatmeal",
                "title = \"Oatmeal\"\nservings = 2\n\n[[ingredients]]\nname = \"Oats\"\nquantity = \"100g\"\ncalories = 400\nprotein_g = 20\nfiber_g = 10\nfat_g = 8\ncarbs_g = 60\nalcohol_g = 0\n",
            ),
            (
                "turkey-chili",
                "title = \"Turkey Chili\"\nservings = 4\n\n[[ingredients]]\nname = \"Turkey\"\ncalories = 800\nprotein_g = 80\nfiber_g = 20\nfat_g = 20\ncarbs_g = 40\nalcohol_g = 0\n",
            ),
            (
                "protein-shake",
                "title = \"Protein Shake\"\nservings = 1\n\n[[ingredients]]\nname = \"Whey\"\ncalories = 200\nprotein_g = 30\nfiber_g = 2\nfat_g = 3\ncarbs_g = 10\nalcohol_g = 0\n",
            ),
        ];
        for (name, toml) in foods {
            std::fs::write(dir.path().join(format!("{name}.toml")), toml).unwrap();
        }
        dir
    }

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("Turkey Chili"), "turkeychili");
        assert_eq!(normalize("sour-cream 60g"), "sourcream60g");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("Café"), "café");
    }

    #[test]
    fn test_exact_name_match() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["oatmeal"] }))
            .unwrap();
        assert!(out.contains("1. oatmeal | Oatmeal | 200 cal/serv, 10 protein_g, 5 fiber_g, 4 fat_g, 30 carbs_g, 0 alcohol_g"), "got: {out}");
    }

    #[test]
    fn test_exact_title_match() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["coffee"] }))
            .unwrap();
        assert!(out.contains("coffee | Coffee | 12 cal/serv"));
    }

    #[test]
    fn test_containment_fallback() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["turkey"] }))
            .unwrap();
        assert!(out.contains("turkey-chili | Turkey Chili | 200 cal/serv"));
    }

    #[test]
    fn test_query_contained_in_name() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["shake"] }))
            .unwrap();
        assert!(out.contains("protein-shake"));
    }

    #[test]
    fn test_unicode_normalization_matches() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("cafe.toml"),
            "title = \"Café au lait\"\nservings = 1\n\n[[ingredients]]\nname = \"Milk\"\ncalories = 100\nprotein_g = 5\nfiber_g = 0\nfat_g = 5\ncarbs_g = 10\nalcohol_g = 0\n",
        )
        .unwrap();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["Café"] }))
            .unwrap();
        assert!(out.contains("cafe | Café au lait"));
    }

    #[test]
    fn test_no_match_returns_empty_result() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["ghost food"] }))
            .unwrap();
        assert!(out.contains("no matches"));
    }

    #[test]
    fn test_empty_foods_dir_returns_no_matches() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["coffee"] }))
            .unwrap();
        assert!(out.contains("no matches"));
    }

    #[test]
    fn test_batch_mode() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["coffee", "oatmeal"] }))
            .unwrap();
        assert!(out.contains("query: coffee"));
        assert!(out.contains("query: oatmeal"));
        assert!(out.contains("coffee | Coffee | 12 cal/serv"));
        assert!(out.contains("oatmeal | Oatmeal | 200 cal/serv"));
    }

    #[test]
    fn test_exact_matches_ranked_first() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["chili"] }))
            .unwrap();
        let exact_pos = out
            .find("turkey-chili | Turkey Chili | 200 cal/serv")
            .unwrap();
        let first_line = out.lines().nth(1).unwrap();
        assert!(first_line.contains("turkey-chili"), "got: {first_line}");
        assert!(exact_pos < out.len());
    }

    #[test]
    fn test_top_five_cap() {
        let dir = tempfile::TempDir::new().unwrap();
        for i in 0..8 {
            std::fs::write(
                dir.path().join(format!("soup-{i}.toml")),
                format!("title = \"Soup {i}\"\nservings = 1\n\n[[ingredients]]\nname = \"Water\"\ncalories = {i}\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n"),
            )
            .unwrap();
        }
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["soup"] }))
            .unwrap();
        let matches = out
            .lines()
            .filter(|l| l.starts_with("  ") && l.contains("| Soup"))
            .count();
        assert_eq!(matches, 5);
    }

    #[test]
    fn test_broken_food_files_skipped_quietly_in_lookup() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("broken.toml"), "not toml at all").unwrap();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["coffee"] }))
            .unwrap();
        assert!(out.contains("no matches"));
    }

    #[test]
    fn test_broken_food_files_warn_through_hook() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("broken.toml"), "not toml at all").unwrap();
        let warned = {
            let warned = std::cell::RefCell::new(Vec::new());
            let warn = |m: &str| warned.borrow_mut().push(m.to_string());
            let tool = FoodLookup::new(dir.path()).with_warn(&warn);
            let out = tool
                .execute(&serde_json::json!({ "queries": ["coffee"] }))
                .unwrap();
            assert!(out.contains("no matches"));
            warned.into_inner()
        };
        assert_eq!(warned.len(), 1);
        assert!(warned[0].contains("broken.toml"), "got: {warned:?}");
    }

    #[test]
    fn test_requires_queries() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        assert!(tool.execute(&serde_json::json!({})).is_err());
        assert!(tool.execute(&serde_json::json!({ "queries": [] })).is_err());
    }

    #[test]
    fn test_empty_query_no_match() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": [""] }))
            .unwrap();
        assert!(out.contains("no matches"));
    }

    #[test]
    fn test_servings_fractional_in_catalog_line() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("half.toml"),
            "title = \"Half\"\nservings = 3\n\n[[ingredients]]\nname = \"X\"\ncalories = 10\nprotein_g = 1\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n",
        )
        .unwrap();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["half"] }))
            .unwrap();
        assert!(out.contains("3.333 cal/serv"), "got: {out}");
    }
}
