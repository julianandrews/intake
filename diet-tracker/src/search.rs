use anyhow::{Context, Result};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FillSelection {
    pub title: String,
    pub servings: u32,
}

#[derive(Debug, Clone)]
pub struct FillResult {
    pub selections: Vec<FillSelection>,
    pub total_calories: u32,
    pub total_protein_g: f64,
    pub total_fiber_g: f64,
}

pub struct FillConfig<'a> {
    pub max_calories: u32,
    pub min_protein_g: f64,
    pub min_fiber_g: f64,
    pub max_servings_per_recipe: u32,
    pub limit: Option<usize>,
    pub exclude: &'a [String],
    pub include: &'a [String],
    pub foods_dir: &'a Path,
    pub max_nodes: u64,
    pub max_results: usize,
}

struct RecipeInfo {
    slug: String,
    title: String,
    cal: u32,
    prot: f64,
    fib: f64,
}

pub fn find_fills(config: &FillConfig) -> Result<Vec<FillResult>> {
    let recipes = crate::recipe::find_all_recipes(config.foods_dir)?;

    let mut info: Vec<RecipeInfo> = recipes
        .into_iter()
        .filter(|(path, _)| {
            let slug = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            !config.exclude.iter().any(|e| e == slug)
                || config.include.iter().any(|e| e == slug)
        })
        .map(|(path, recipe)| {
            let ps = recipe.per_serving();
            RecipeInfo {
                slug: path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string(),
                title: recipe.title,
                cal: ps.calories,
                prot: ps.protein_g,
                fib: ps.fiber_g,
            }
        })
        .collect();

    for inc in config.include {
        if !info.iter().any(|r| r.slug == *inc) {
            anyhow::bail!("included recipe '{}' not found", inc);
        }
    }

    info.sort_by(|a, b| {
        let a_inc = config.include.iter().any(|e| e == &a.slug);
        let b_inc = config.include.iter().any(|e| e == &b.slug);
        let eff_a = if a.cal > 0 { (a.prot + a.fib) / a.cal as f64 } else { f64::MAX };
        let eff_b = if b.cal > 0 { (b.prot + b.fib) / b.cal as f64 } else { f64::MAX };
        match (a_inc, b_inc) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => eff_b.partial_cmp(&eff_a).unwrap_or(std::cmp::Ordering::Equal),
        }
    });

    let n = info.len();
    let mut max_prot_per_cal = vec![0.0; n];
    let mut max_fib_per_cal = vec![0.0; n];
    for i in (0..n).rev() {
        let r = &info[i];
        let ppc = if r.cal > 0 { r.prot / r.cal as f64 } else { 0.0 };
        let fpc = if r.cal > 0 { r.fib / r.cal as f64 } else { 0.0 };
        max_prot_per_cal[i] = ppc.max(*max_prot_per_cal.get(i + 1).unwrap_or(&0.0));
        max_fib_per_cal[i] = fpc.max(*max_fib_per_cal.get(i + 1).unwrap_or(&0.0));
    }

    let mut solutions = Vec::new();
    let mut nodes: u64 = 0;

    struct State {
        selections: Vec<FillSelection>,
        cal: u32,
        prot: f64,
        fib: f64,
    }

    fn backtrack(
        idx: usize,
        state: &mut State,
        config: &FillConfig,
        info: &[RecipeInfo],
        max_prot_per_cal: &[f64],
        max_fib_per_cal: &[f64],
        solutions: &mut Vec<FillResult>,
        nodes: &mut u64,
    ) -> Result<()> {
        *nodes += 1;
        if *nodes > config.max_nodes || solutions.len() >= config.max_results {
            return Ok(());
        }

        if idx == info.len() {
            if state.prot >= config.min_protein_g && state.fib >= config.min_fiber_g {
                solutions.push(FillResult {
                    selections: state.selections.clone(),
                    total_calories: state.cal,
                    total_protein_g: state.prot,
                    total_fiber_g: state.fib,
                });
            }
            return Ok(());
        }

        let rem_cal = config.max_calories.saturating_sub(state.cal);
        if state.prot + rem_cal as f64 * max_prot_per_cal[idx] < config.min_protein_g {
            return Ok(());
        }
        if state.fib + rem_cal as f64 * max_fib_per_cal[idx] < config.min_fiber_g {
            return Ok(());
        }

        let r = &info[idx];
        let max_s = if r.cal > 0 {
            std::cmp::min(rem_cal / r.cal, config.max_servings_per_recipe)
        } else {
            0
        };
        let min_s = if config.include.iter().any(|e| e == &r.slug) { 1 } else { 0 };

        for s in (min_s..=max_s).rev() {
            let cal_delta = s.checked_mul(r.cal).context("calorie calculation overflow")?;
            let new_cal = state.cal.checked_add(cal_delta).context("calorie calculation overflow")?;
            if new_cal > config.max_calories {
                continue;
            }

            if s > 0 {
                state.selections.push(FillSelection {
                    title: r.title.clone(),
                    servings: s,
                });
                state.cal = new_cal;
                state.prot += s as f64 * r.prot;
                state.fib += s as f64 * r.fib;
            }

            backtrack(
                idx + 1, state, config, info,
                max_prot_per_cal, max_fib_per_cal,
                solutions, nodes,
            )?;

            if s > 0 {
                state.selections.pop();
                state.cal = state.cal.checked_sub(cal_delta).context("calorie calculation underflow")?;
                state.prot -= s as f64 * r.prot;
                state.fib -= s as f64 * r.fib;
            }
        }

        Ok(())
    }

    let mut state = State {
        selections: Vec::new(),
        cal: 0,
        prot: 0.0,
        fib: 0.0,
    };

    backtrack(
        0, &mut state, config, &info,
        &max_prot_per_cal, &max_fib_per_cal,
        &mut solutions, &mut nodes,
    )?;

    solutions.sort_by(|a, b| {
        let a_ok = a.total_protein_g >= config.min_protein_g
            && a.total_fiber_g >= config.min_fiber_g;
        let b_ok = b.total_protein_g >= config.min_protein_g
            && b.total_fiber_g >= config.min_fiber_g;
        match (a_ok, b_ok) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => {
                let a_items = a.selections.len();
                let b_items = b.selections.len();
                match a_items.cmp(&b_items) {
                    std::cmp::Ordering::Equal => {
                        let a_dist = config.max_calories - a.total_calories;
                        let b_dist = config.max_calories - b.total_calories;
                        a_dist.cmp(&b_dist)
                    }
                    other => other,
                }
            }
        }
    });

    if let Some(n) = config.limit {
        solutions.truncate(n);
    }

    Ok(solutions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn temp_recipe(dir: &Path, slug: &str, title: &str, servings: u32, cal: u32, prot: f64, fib: f64) {
        let path = dir.join(format!("{}.toml", slug));
        let toml = format!(
            r#"title = "{}"
servings = {}

[[ingredients]]
name = "Test Ingredient"
protein_g = {}
fiber_g = {}
calories = {}
"#,
            title, servings, prot, fib, cal
        );
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(toml.as_bytes()).unwrap();
    }

    #[test]
    fn test_basic_fill() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        temp_recipe(dir.path(), "item-a", "Item A", 1, 200, 20.0, 5.0);
        temp_recipe(dir.path(), "item-b", "Item B", 1, 100, 5.0, 2.0);

        let config = FillConfig {
            max_calories: 300,
            min_protein_g: 20.0,
            min_fiber_g: 5.0,
            max_servings_per_recipe: 3,
            limit: None,
            exclude: &[],
            include: &[],
            foods_dir: dir.path(),
            max_nodes: 100_000,
            max_results: 1000,
        };

        let results = find_fills(&config)?;
        assert!(!results.is_empty(), "should find at least one combo");

        let best = &results[0];
        assert!(best.total_calories <= 300);
        assert!(best.total_protein_g >= 20.0);
        assert!(best.total_fiber_g >= 5.0);

        Ok(())
    }

    #[test]
    fn test_no_solution() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        temp_recipe(dir.path(), "item-a", "Item A", 1, 200, 1.0, 0.0);

        let config = FillConfig {
            max_calories: 200,
            min_protein_g: 50.0,
            min_fiber_g: 10.0,
            max_servings_per_recipe: 3,
            limit: None,
            exclude: &[],
            include: &[],
            foods_dir: dir.path(),
            max_nodes: 100_000,
            max_results: 1000,
        };

        let results = find_fills(&config)?;
        assert!(results.is_empty(), "should find no combos");

        Ok(())
    }

    #[test]
    fn test_exclude_recipe() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        temp_recipe(dir.path(), "item-a", "Item A", 1, 200, 20.0, 5.0);
        temp_recipe(dir.path(), "item-b", "Item B", 1, 100, 30.0, 8.0);

        let exclude = vec!["item-b".to_string()];
        let config = FillConfig {
            max_calories: 300,
            min_protein_g: 20.0,
            min_fiber_g: 5.0,
            max_servings_per_recipe: 3,
            limit: None,
            exclude: &exclude,
            include: &[],
            foods_dir: dir.path(),
            max_nodes: 100_000,
            max_results: 1000,
        };

        let results = find_fills(&config)?;
        for r in &results {
            for s in &r.selections {
                assert_ne!(s.title, "Item B", "item-b should be excluded");
            }
        }

        Ok(())
    }

    #[test]
    fn test_max_servings_cap() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        temp_recipe(dir.path(), "low-cal", "Low Cal", 1, 50, 5.0, 1.0);

        let config = FillConfig {
            max_calories: 500,
            min_protein_g: 5.0,
            min_fiber_g: 1.0,
            max_servings_per_recipe: 2,
            limit: None,
            exclude: &[],
            include: &[],
            foods_dir: dir.path(),
            max_nodes: 100_000,
            max_results: 1000,
        };

        let results = find_fills(&config)?;
        for r in &results {
            for s in &r.selections {
                assert!(s.servings <= 2, "servings should be capped at 2");
            }
        }

        Ok(())
    }

    #[test]
    fn test_limit() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        temp_recipe(dir.path(), "item-a", "Item A", 1, 100, 10.0, 2.0);
        temp_recipe(dir.path(), "item-b", "Item B", 1, 100, 10.0, 2.0);

        let config = FillConfig {
            max_calories: 300,
            min_protein_g: 5.0,
            min_fiber_g: 1.0,
            max_servings_per_recipe: 3,
            limit: Some(3),
            exclude: &[],
            include: &[],
            foods_dir: dir.path(),
            max_nodes: 100_000,
            max_results: 1000,
        };

        let results = find_fills(&config)?;
        assert_eq!(results.len(), 3, "should be limited to 3 results");

        Ok(())
    }

    #[test]
    fn test_include_recipe() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        temp_recipe(dir.path(), "must-have", "Must Have", 1, 100, 10.0, 2.0);
        temp_recipe(dir.path(), "extra", "Extra", 1, 100, 5.0, 1.0);

        let include = vec!["must-have".to_string()];
        let config = FillConfig {
            max_calories: 300,
            min_protein_g: 5.0,
            min_fiber_g: 1.0,
            max_servings_per_recipe: 3,
            limit: None,
            exclude: &[],
            include: &include,
            foods_dir: dir.path(),
            max_nodes: 100_000,
            max_results: 1000,
        };

        let results = find_fills(&config)?;
        assert!(!results.is_empty(), "should find combos with included recipe");
        for r in &results {
            let has_included = r.selections.iter().any(|s| s.title == "Must Have");
            assert!(has_included, "every solution must include 'Must Have'");
        }

        Ok(())
    }

    #[test]
    fn test_include_overrides_exclude() -> Result<()> {
        let dir = tempfile::TempDir::new()?;

        temp_recipe(dir.path(), "item", "Item", 1, 100, 10.0, 2.0);

        let exclude = vec!["item".to_string()];
        let include = vec!["item".to_string()];
        let config = FillConfig {
            max_calories: 200,
            min_protein_g: 5.0,
            min_fiber_g: 1.0,
            max_servings_per_recipe: 3,
            limit: None,
            exclude: &exclude,
            include: &include,
            foods_dir: dir.path(),
            max_nodes: 100_000,
            max_results: 1000,
        };

        let results = find_fills(&config)?;
        assert!(!results.is_empty(), "include should override exclude");

        Ok(())
    }
}
