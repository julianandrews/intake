use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Goals {
    pub max_calories: u32,
    pub min_protein: f64,
    pub min_fiber: f64,
}

#[derive(Debug, Deserialize)]
pub struct SearchConfig {
    #[serde(default = "default_max_nodes")]
    pub max_nodes: u64,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_max_nodes() -> u64 {
    100_000
}

fn default_max_results() -> usize {
    1000
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            max_nodes: default_max_nodes(),
            max_results: default_max_results(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    pub(crate) goals: Goals,
    #[serde(default)]
    pub(crate) search: SearchConfig,
}

#[allow(dead_code)]
pub fn load_goals(path: &Path) -> Result<Goals> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading config: {}", path.display()))?;
    let config: Config =
        toml::from_str(&content).with_context(|| format!("parsing config: {}", path.display()))?;
    Ok(config.goals)
}

pub fn load_config(path: &Path) -> Result<Config> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading config: {}", path.display()))?;
    let config: Config =
        toml::from_str(&content).with_context(|| format!("parsing config: {}", path.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_goals() -> Result<()> {
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join("config.toml");
        let toml = r#"
[goals]
max_calories = 2500
min_protein = 150
min_fiber = 30
"#;
        std::fs::write(&path, toml)?;
        let goals = load_goals(&path)?;
        assert_eq!(goals.max_calories, 2500);
        assert!((goals.min_protein - 150.0).abs() < 0.001);
        assert!((goals.min_fiber - 30.0).abs() < 0.001);
        Ok(())
    }

    #[test]
    fn test_load_goals_missing_file() {
        let result = load_goals(Path::new("/nonexistent/path.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_search_config_defaults() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let toml = r#"
[goals]
max_calories = 2000
min_protein = 100
min_fiber = 20
"#;
        std::fs::write(&path, toml).unwrap();
        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.search.max_nodes, 100_000);
        assert_eq!(cfg.search.max_results, 1000);
    }

    #[test]
    fn test_search_config_custom() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let toml = r#"
[goals]
max_calories = 2000
min_protein = 100
min_fiber = 20

[search]
max_nodes = 50000
max_results = 50
"#;
        std::fs::write(&path, toml).unwrap();
        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.search.max_nodes, 50000);
        assert_eq!(cfg.search.max_results, 50);
    }
}
