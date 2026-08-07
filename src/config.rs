use crate::display::{ColumnTarget, DayTargets};
use anyhow::{bail, Context, Result};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::path::PathBuf;

pub const DEFAULT_COLUMNS: &[Column] = &[
    Column::Calories,
    Column::Carbs,
    Column::Fat,
    Column::Protein,
    Column::Fiber,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    Calories,
    Protein,
    Fiber,
    Fat,
    Carbs,
    Alcohol,
}

impl Column {
    pub fn all() -> [Column; 6] {
        [
            Column::Calories,
            Column::Protein,
            Column::Fiber,
            Column::Fat,
            Column::Carbs,
            Column::Alcohol,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Column::Calories => "Calories",
            Column::Protein => "Protein(g)",
            Column::Fiber => "Fiber(g)",
            Column::Fat => "Fat(g)",
            Column::Carbs => "Carbs(g)",
            Column::Alcohol => "Alcohol(g)",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub foods_dir: Option<PathBuf>,
    pub log_dir: Option<PathBuf>,
    pub max_calories: Option<u32>,
    pub min_protein: Option<Decimal>,
    pub min_fiber: Option<Decimal>,
    pub maintenance_calories: Option<u32>,
    pub show_columns: Option<Vec<Column>>,
    pub min_calories: Option<Decimal>,
    pub max_protein: Option<Decimal>,
    pub max_fiber: Option<Decimal>,
    pub min_fat: Option<Decimal>,
    pub max_fat: Option<Decimal>,
    pub min_carbs: Option<Decimal>,
    pub max_carbs: Option<Decimal>,
    pub min_alcohol: Option<Decimal>,
    pub max_alcohol: Option<Decimal>,
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

    pub fn columns(&self) -> Result<Vec<Column>> {
        let Some(columns) = &self.show_columns else {
            return Ok(DEFAULT_COLUMNS.to_vec());
        };
        for (i, column) in columns.iter().enumerate() {
            if columns[..i].contains(column) {
                bail!(
                    "show_columns contains duplicate column '{}'",
                    format!("{column:?}").to_lowercase()
                );
            }
        }
        Ok(columns.clone())
    }

    pub fn targets(&self) -> Result<DayTargets> {
        let targets = DayTargets {
            calories: ColumnTarget {
                min: self.min_calories,
                max: self.max_calories.map(Decimal::from),
            },
            protein: ColumnTarget {
                min: self.min_protein,
                max: self.max_protein,
            },
            fiber: ColumnTarget {
                min: self.min_fiber,
                max: self.max_fiber,
            },
            fat: ColumnTarget {
                min: self.min_fat,
                max: self.max_fat,
            },
            carbs: ColumnTarget {
                min: self.min_carbs,
                max: self.max_carbs,
            },
            alcohol: ColumnTarget {
                min: self.min_alcohol,
                max: self.max_alcohol,
            },
        };
        for (name, target) in [
            ("calories", targets.calories),
            ("protein", targets.protein),
            ("fiber", targets.fiber),
            ("fat", targets.fat),
            ("carbs", targets.carbs),
            ("alcohol", targets.alcohol),
        ] {
            if let Some(min) = target.min {
                if min < Decimal::ZERO {
                    bail!("{name} target min ({min}) must be non-negative");
                }
            }
            if let Some(max) = target.max {
                if max < Decimal::ZERO {
                    bail!("{name} target max ({max}) must be non-negative");
                }
            }
            if let (Some(min), Some(max)) = (target.min, target.max) {
                if min > max {
                    bail!("{name} target min ({min}) exceeds max ({max})");
                }
            }
        }
        Ok(targets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn test_columns_default_excludes_alcohol() {
        let config = Config::default();
        assert_eq!(
            config.columns().unwrap(),
            vec![
                Column::Calories,
                Column::Carbs,
                Column::Fat,
                Column::Protein,
                Column::Fiber
            ]
        );
    }

    #[test]
    fn test_columns_from_config() {
        let config: Config =
            toml::from_str("show_columns = [\"calories\", \"fat\", \"alcohol\"]\n").unwrap();
        assert_eq!(
            config.columns().unwrap(),
            vec![Column::Calories, Column::Fat, Column::Alcohol]
        );
    }

    #[test]
    fn test_columns_empty_allowed() {
        let config: Config = toml::from_str("show_columns = []\n").unwrap();
        assert!(config.columns().unwrap().is_empty());
    }

    #[test]
    fn test_columns_duplicates_rejected() {
        let config: Config =
            toml::from_str("show_columns = [\"fat\", \"fat\", \"calories\", \"fat\"]\n").unwrap();
        let err = config.columns().unwrap_err().to_string();
        assert!(err.contains("duplicate column"), "unexpected error: {err}");
    }

    #[test]
    fn test_targets_from_flat_keys() {
        let config: Config =
            toml::from_str("max_calories = 2000\nmin_protein = 100\nmin_fat = 50\nmax_fat = 90\n")
                .unwrap();
        let targets = config.targets().unwrap();
        assert_eq!(
            targets.calories,
            ColumnTarget {
                min: None,
                max: Some(Decimal::from(2000))
            }
        );
        assert_eq!(
            targets.fat,
            ColumnTarget {
                min: Some(Decimal::from(50)),
                max: Some(Decimal::from(90))
            }
        );
        assert_eq!(
            targets.alcohol,
            ColumnTarget {
                min: None,
                max: None
            }
        );
    }

    #[test]
    fn test_targets_min_exceeds_max_rejected() {
        let config: Config = toml::from_str("min_fat = 90\nmax_fat = 50\n").unwrap();
        let err = config.targets().unwrap_err().to_string();
        assert!(err.contains("fat target min"), "unexpected error: {err}");
    }

    #[test]
    fn test_targets_negative_rejected() {
        let config: Config = toml::from_str("max_protein = -10\n").unwrap();
        let err = config.targets().unwrap_err().to_string();
        assert!(err.contains("non-negative"), "unexpected error: {err}");

        let config: Config = toml::from_str("min_fat = -1.5\n").unwrap();
        let err = config.targets().unwrap_err().to_string();
        assert!(err.contains("non-negative"), "unexpected error: {err}");
    }

    #[test]
    fn test_targets_valid_band_accepted() {
        let config: Config = toml::from_str("min_carbs = 100\nmax_carbs = 300\n").unwrap();
        assert!(config.targets().is_ok());
    }

    #[test]
    fn test_unknown_column_rejected() {
        assert!(toml::from_str::<Config>("show_columns = [\"bogus\"]\n").is_err());
    }
}
