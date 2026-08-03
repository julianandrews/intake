use anyhow::{Context, Result};
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
    pub fn resolve(foods_dir: Option<PathBuf>, log_dir: Option<PathBuf>) -> Result<Self> {
        let mut config = Self::load()?;

        if let Ok(val) = std::env::var("INTAKE_FOODS_DIR") {
            config.foods_dir = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("INTAKE_LOG_DIR") {
            config.log_dir = Some(PathBuf::from(val));
        }

        if let Some(dir) = foods_dir {
            config.foods_dir = Some(dir);
        }
        if let Some(dir) = log_dir {
            config.log_dir = Some(dir);
        }

        Ok(config)
    }

    fn load() -> Result<Self> {
        let config_path = dirs::config_dir().map(|p| p.join("intake").join("config.toml"));

        match config_path {
            Some(path) if path.exists() => {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read config: {}", path.display()))?;
                Ok(toml::from_str(&content)?)
            }
            _ => Ok(Config::default()),
        }
    }

    pub fn foods_dir(&self) -> PathBuf {
        self.foods_dir.clone().unwrap_or_else(|| {
            dirs::data_dir()
                .map(|p| p.join("intake").join("foods"))
                .unwrap_or_else(|| PathBuf::from("foods"))
        })
    }

    pub fn log_dir(&self) -> PathBuf {
        self.log_dir.clone().unwrap_or_else(|| {
            dirs::data_dir()
                .map(|p| p.join("intake").join("log"))
                .unwrap_or_else(|| PathBuf::from("log"))
        })
    }
}
