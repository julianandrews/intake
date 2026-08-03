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
    pub fn resolve() -> Result<Self> {
        let mut config = Self::load()?;
        config.apply_env_overrides();
        Ok(config)
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

    pub fn with_cli_overrides(
        mut self,
        foods_dir: Option<PathBuf>,
        log_dir: Option<PathBuf>,
    ) -> Self {
        if let Some(dir) = foods_dir {
            self.foods_dir = Some(dir);
        }
        if let Some(dir) = log_dir {
            self.log_dir = Some(dir);
        }
        self
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
