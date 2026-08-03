use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub foods_dir: Option<PathBuf>,
    pub log_dir: Option<PathBuf>,
    pub max_calories: Option<u32>,
    pub min_protein: Option<f64>,
    pub min_fiber: Option<f64>,
    pub maintenance_calories: Option<u32>,
}

impl Config {
    pub fn resolve() -> Self {
        let mut config = Self::load().unwrap_or_default();
        config.apply_env_overrides();
        config
    }

    fn load() -> Result<Self> {
        let config_path = dirs::config_dir().map(|p| p.join("intake").join("config.toml"));

        match config_path {
            Some(path) if path.exists() => {
                let content = std::fs::read_to_string(&path)?;
                Ok(toml::from_str(&content)?)
            }
            _ => Ok(Config::default()),
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(val) = std::env::var("INTAKE_FOODS_DIR") {
            self.foods_dir = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("INTAKE_LOG_DIR") {
            self.log_dir = Some(PathBuf::from(val));
        }
    }
}
