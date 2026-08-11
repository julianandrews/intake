use crate::food;
use intake_ai::tools::Tool;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::Path;

use super::catalog;

fn eprintln_warn(msg: &str) {
    eprintln!("{msg}");
}

const MAX_PER_QUERY: usize = 5;
const TOTAL_CAP: usize = 2000;

/// Lowercase, fold diacritics (`é`→`e`), strip non-alphanumerics.
fn normalize(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        for lower in c.to_lowercase() {
            if lower.is_alphanumeric() {
                match fold_char(lower) {
                    Some(folded) => out.push_str(folded),
                    None => out.push(lower),
                }
            }
        }
    }
    out
}

/// Fold a single accented or special Latin character to its ASCII base.
fn fold_char(c: char) -> Option<&'static str> {
    Some(match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' | 'ǎ' => "a",
        'é' | 'è' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
        'í' | 'ì' | 'î' | 'ï' | 'ī' | 'į' | 'ǐ' | 'ı' => "i",
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'ǒ' => "o",
        'ú' | 'ù' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ũ' | 'ű' | 'ǔ' => "u",
        'ý' | 'ÿ' | 'ŷ' | 'ỳ' | 'ỵ' | 'ỷ' | 'ỹ' => "y",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ñ' | 'ń' | 'ň' | 'ņ' => "n",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => "s",
        'ź' | 'ż' | 'ž' => "z",
        'ğ' | 'ģ' => "g",
        'ł' | 'ļ' | 'ľ' => "l",
        'ř' | 'ŕ' => "r",
        'ţ' | 'ť' | 'ț' => "t",
        'đ' | 'ď' => "d",
        'ķ' => "k",
        'ħ' => "h",
        'ŵ' => "w",
        'ẋ' => "x",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        'ð' => "d",
        'þ' => "th",
        _ => return None,
    })
}

/// Fold a whole word's diacritics in place.
fn fold_word(w: &str) -> String {
    let mut out = String::new();
    for c in w.chars() {
        match fold_char(c) {
            Some(folded) => out.push_str(folded),
            None => out.push(c),
        }
    }
    out
}

/// Lowercase, diacritic-folded words; word boundaries survive.
fn words(s: &str) -> Vec<String> {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(|w| fold_word(&w.to_lowercase()))
        .collect()
}

fn bigrams(s: &str) -> Vec<(char, char)> {
    let chars: Vec<char> = s.chars().collect();
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}

/// Scaled Dice coefficient over character bigrams, pure integer: returns
/// `(2 * |common|, |Bq| + |Bt|)`; equal when either side has no bigrams.
fn dice(q_bigrams: &HashSet<(char, char)>, t: &str) -> (usize, usize) {
    let t_bigrams: HashSet<(char, char)> = bigrams(t).into_iter().collect();
    if q_bigrams.is_empty() || t_bigrams.is_empty() {
        return (0, 0);
    }
    let common = q_bigrams.iter().filter(|b| t_bigrams.contains(b)).count();
    (2 * common, q_bigrams.len() + t_bigrams.len())
}

struct Score {
    exact: bool,
    tokens: usize,
    dice_num: usize,
    dice_den: usize,
}

/// Tiered per-food score: exact > token overlap > bigram Dice.
fn score(
    q: &str,
    q_words: &[String],
    q_bigrams: &HashSet<(char, char)>,
    name: &str,
    title: &str,
) -> Score {
    let norm_name = normalize(name);
    let norm_title = normalize(title);
    let exact = norm_name == q || norm_title == q;
    let name_words = words(name);
    let title_words = words(title);
    let tokens = q_words
        .iter()
        .filter(|qw| {
            name_words
                .iter()
                .any(|w| w == *qw || w.starts_with(qw.as_str()))
                || title_words
                    .iter()
                    .any(|w| w == *qw || w.starts_with(qw.as_str()))
        })
        .count();
    let (n1, d1) = dice(q_bigrams, &norm_name);
    let (n2, d2) = dice(q_bigrams, &norm_title);
    let (dice_num, dice_den) = if n1 as u128 * d2 as u128 >= n2 as u128 * d1 as u128 {
        (n1, d1)
    } else {
        (n2, d2)
    };
    Score {
        exact,
        tokens,
        dice_num,
        dice_den,
    }
}

/// Lexicographic score comparison; Dice ratios compared by integer
/// cross-multiplication (never floating point).
fn compare_scores(a: &Score, b: &Score) -> Ordering {
    b.exact
        .cmp(&a.exact)
        .then(b.tokens.cmp(&a.tokens))
        .then_with(|| {
            let ab = a.dice_num as u128 * b.dice_den as u128;
            let ba = b.dice_num as u128 * a.dice_den as u128;
            ba.cmp(&ab)
        })
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
        let q_words = words(query);
        let q_bigrams: HashSet<(char, char)> = bigrams(&q).into_iter().collect();

        let mut scored: Vec<(&String, &food::Food, Score)> = Vec::new();
        for (name, f) in catalog {
            let s = score(&q, &q_words, &q_bigrams, name, &f.title);
            if s.exact || s.tokens > 0 || s.dice_num > 0 {
                scored.push((name, f, s));
            }
        }
        scored.sort_by(|a, b| compare_scores(&a.2, &b.2).then_with(|| a.0.cmp(b.0)));

        let mut seen: HashSet<String> = HashSet::new();
        let mut count = 0usize;
        for (name, f, _) in scored {
            if !seen.insert((*name).clone()) {
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
        assert_eq!(normalize("Café"), "cafe");
        assert_eq!(normalize("Jalapeño"), "jalapeno");
        assert_eq!(normalize("Strøganoff"), "stroganoff");
    }

    #[test]
    fn test_words_split() {
        assert_eq!(
            words("Turkey Chili"),
            vec!["turkey".to_string(), "chili".to_string()]
        );
        assert_eq!(
            words("sour-cream 60g"),
            vec!["sour".to_string(), "cream".to_string(), "60g".to_string()]
        );
        assert_eq!(
            words("Café au lait"),
            vec!["cafe".to_string(), "au".to_string(), "lait".to_string()]
        );
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
    fn test_diacritic_folding_ascii_query() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("cafe.toml"),
            "title = \"Café au lait\"\nservings = 1\n\n[[ingredients]]\nname = \"Milk\"\ncalories = 100\nprotein_g = 5\nfiber_g = 0\nfat_g = 5\ncarbs_g = 10\nalcohol_g = 0\n",
        )
        .unwrap();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["cafe"] }))
            .unwrap();
        assert!(out.contains("cafe | Café au lait"));
    }

    #[test]
    fn test_typo_matches_via_bigrams() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["chilli"] }))
            .unwrap();
        assert!(out.contains("turkey-chili | Turkey Chili"), "got: {out}");
    }

    #[test]
    fn test_word_order_matches() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["chili turkey"] }))
            .unwrap();
        assert!(out.contains("turkey-chili | Turkey Chili"), "got: {out}");
    }

    #[test]
    fn test_token_overlap_ranks_relevance() {
        let dir = tempfile::TempDir::new().unwrap();
        for (name, title) in [
            ("mushroom-soup", "Mushroom Soup"),
            ("chicken-soup", "Chicken Soup"),
            ("chicken-sandwich", "Chicken Sandwich"),
        ] {
            std::fs::write(
                dir.path().join(format!("{name}.toml")),
                format!("title = \"{title}\"\nservings = 1\n\n[[ingredients]]\nname = \"X\"\ncalories = 100\nprotein_g = 1\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n"),
            )
            .unwrap();
        }
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["chicken soup"] }))
            .unwrap();
        let first = out.lines().nth(1).unwrap();
        assert!(first.contains("chicken-soup"), "got: {out}");
    }

    #[test]
    fn test_single_char_query_prefix_matches() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["o"] }))
            .unwrap();
        assert!(out.contains("oatmeal | Oatmeal"), "got: {out}");
    }

    #[test]
    fn test_gibberish_returns_no_matches() {
        let dir = foods_dir();
        let tool = FoodLookup::new(dir.path());
        let out = tool
            .execute(&serde_json::json!({ "queries": ["zzzqq"] }))
            .unwrap();
        assert!(out.contains("no matches"));
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
