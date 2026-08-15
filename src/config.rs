use crate::amount::Calories;
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

/// The display format for the day view's Time column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum TimeFormat {
    /// 24-hour, `HH:MM` zero-padded (e.g. `14:05`).
    #[serde(rename = "24h")]
    H24,
    /// 12-hour, `h:mm AM/PM` (e.g. `2:05 PM`).
    #[serde(rename = "12h")]
    H12,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ColumnTarget {
    pub min: Option<Decimal>,
    pub max: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DayTargets {
    pub calories: ColumnTarget,
    pub protein: ColumnTarget,
    pub fiber: ColumnTarget,
    pub fat: ColumnTarget,
    pub carbs: ColumnTarget,
    pub alcohol: ColumnTarget,
}

impl DayTargets {
    pub fn for_column(&self, column: Column) -> ColumnTarget {
        match column {
            Column::Calories => self.calories,
            Column::Protein => self.protein,
            Column::Fiber => self.fiber,
            Column::Fat => self.fat,
            Column::Carbs => self.carbs,
            Column::Alcohol => self.alcohol,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub foods_dir: Option<PathBuf>,
    pub log_dir: Option<PathBuf>,
    pub max_calories: Option<Calories>,
    pub min_protein: Option<Decimal>,
    pub min_fiber: Option<Decimal>,
    pub maintenance_calories: Option<Calories>,
    pub show_columns: Option<Vec<Column>>,
    pub min_calories: Option<Calories>,
    pub max_protein: Option<Decimal>,
    pub max_fiber: Option<Decimal>,
    pub min_fat: Option<Decimal>,
    pub max_fat: Option<Decimal>,
    pub min_carbs: Option<Decimal>,
    pub max_carbs: Option<Decimal>,
    pub min_alcohol: Option<Decimal>,
    pub max_alcohol: Option<Decimal>,
    pub write_timestamps: Option<bool>,
    pub show_timestamp: Option<bool>,
    pub time_format: Option<TimeFormat>,
    pub summary_days: Option<u32>,
    #[cfg(feature = "ai")]
    pub ai: Option<crate::ai::settings::AiConfig>,
}

impl Config {
    pub fn resolve(foods_dir: Option<PathBuf>, log_dir: Option<PathBuf>) -> Result<Self> {
        let mut config = Self::load()?;

        if let Ok(val) = std::env::var("INTAKE_FOODS_DIR") {
            if !val.is_empty() {
                config.foods_dir = Some(PathBuf::from(val));
            }
        }
        if let Ok(val) = std::env::var("INTAKE_LOG_DIR") {
            if !val.is_empty() {
                config.log_dir = Some(PathBuf::from(val));
            }
        }

        if let Some(dir) = foods_dir {
            config.foods_dir = Some(dir);
        }
        if let Some(dir) = log_dir {
            config.log_dir = Some(dir);
        }

        if config
            .foods_dir
            .as_ref()
            .is_some_and(|d| d.as_os_str().is_empty())
        {
            bail!("foods_dir must not be empty");
        }
        if config
            .log_dir
            .as_ref()
            .is_some_and(|d| d.as_os_str().is_empty())
        {
            bail!("log_dir must not be empty");
        }

        Ok(config)
    }

    fn load() -> Result<Self> {
        let config_path = dirs::config_dir().map(|p| p.join("intake").join("config.toml"));

        match config_path {
            Some(path) if path.exists() => {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read config: {}", path.display()))?;
                Ok(toml::from_str(&content)
                    .with_context(|| format!("failed to parse config: {}", path.display()))?)
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
                .map(|p| p.join("intake").join("logs"))
                .unwrap_or_else(|| PathBuf::from("logs"))
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

    /// Whether new log entries get a timestamp on write (default: true).
    /// An explicit `--time` / `retime` overrides this per invocation.
    pub fn write_timestamps(&self) -> bool {
        self.write_timestamps.unwrap_or(true)
    }

    /// Whether the day view shows the Time column (default: true).
    pub fn show_timestamp(&self) -> bool {
        self.show_timestamp.unwrap_or(true)
    }

    /// The display format for Time cells (default: 24-hour `HH:MM`). An
    /// unknown `time_format` value is rejected at config parse.
    pub fn time_format(&self) -> TimeFormat {
        self.time_format.unwrap_or(TimeFormat::H24)
    }

    /// The default window length for `summary` (default: 7 days). An
    /// explicit `--days` overrides this per invocation.
    pub fn summary_days(&self) -> u32 {
        self.summary_days.unwrap_or(7)
    }

    pub fn targets(&self) -> Result<DayTargets> {
        let targets = DayTargets {
            calories: ColumnTarget {
                min: self.min_calories.map(Decimal::from),
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
            if let (Some(min), Some(max)) = (target.min, target.max) {
                if min > max {
                    bail!("{name} target min ({min}) exceeds max ({max})");
                }
            }
        }
        // Non-negativity for the macro targets typed as bare `Decimal`;
        // calorie targets are `Calories` and already enforce it at parse.
        for (name, target) in [
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
    fn test_negative_calorie_values_rejected_at_parse() {
        assert!(toml::from_str::<Config>("max_calories = -10\n").is_err());
        assert!(toml::from_str::<Config>("min_calories = -0.5\n").is_err());
        assert!(toml::from_str::<Config>("maintenance_calories = -1\n").is_err());
        assert!(toml::from_str::<Config>("maintenance_calories = 0\n").is_ok());
    }

    #[test]
    fn test_unknown_column_rejected() {
        assert!(toml::from_str::<Config>("show_columns = [\"bogus\"]\n").is_err());
    }

    #[test]
    fn test_timestamp_config_defaults() {
        let config = Config::default();
        assert!(config.write_timestamps());
        assert!(config.show_timestamp());
        assert_eq!(config.time_format(), TimeFormat::H24);
    }

    #[test]
    fn test_timestamp_config_flags_parse() {
        let config: Config =
            toml::from_str("write_timestamps = false\nshow_timestamp = false\n").unwrap();
        assert!(!config.write_timestamps());
        assert!(!config.show_timestamp());
        assert_eq!(config.time_format(), TimeFormat::H24);
    }

    #[test]
    fn test_time_format_12h_parses() {
        let config: Config = toml::from_str("time_format = \"12h\"\n").unwrap();
        assert_eq!(config.time_format(), TimeFormat::H12);
    }

    #[test]
    fn test_time_format_unknown_rejected() {
        let err = toml::from_str::<Config>("time_format = \"bogus\"\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("24h"), "got: {err}");
    }

    #[test]
    fn test_summary_days_default() {
        let config = Config::default();
        assert_eq!(config.summary_days(), 7);
    }

    #[test]
    fn test_summary_days_parses() {
        let config: Config = toml::from_str("summary_days = 3\n").unwrap();
        assert_eq!(config.summary_days(), 3);
    }

    #[test]
    fn test_unknown_config_key_rejected() {
        let err = toml::from_str::<Config>("max_calorys = 1800\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("max_calorys"), "got: {err}");
    }

    #[test]
    fn test_resolve_ignores_empty_env_vars() {
        let original_foods = std::env::var("INTAKE_FOODS_DIR").ok();
        let original_log = std::env::var("INTAKE_LOG_DIR").ok();

        std::env::set_var("INTAKE_FOODS_DIR", "");
        std::env::set_var("INTAKE_LOG_DIR", "");
        let config = Config::resolve(None, None).unwrap();

        match original_foods {
            Some(v) => std::env::set_var("INTAKE_FOODS_DIR", v),
            None => std::env::remove_var("INTAKE_FOODS_DIR"),
        }
        match original_log {
            Some(v) => std::env::set_var("INTAKE_LOG_DIR", v),
            None => std::env::remove_var("INTAKE_LOG_DIR"),
        }

        assert_eq!(config.foods_dir, None);
        assert_eq!(config.log_dir, None);
    }

    #[test]
    fn test_resolve_rejects_empty_cli_paths() {
        assert!(Config::resolve(Some(PathBuf::from("")), None).is_err());
        assert!(Config::resolve(None, Some(PathBuf::from(""))).is_err());
        assert!(Config::resolve(Some(PathBuf::from("foods")), None).is_ok());
    }

    #[cfg(not(feature = "ai"))]
    #[test]
    fn test_ai_table_rejected_without_feature() {
        let err = toml::from_str::<Config>("[ai]\nmodel = \"m\"\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("ai"), "got: {err}");
    }
}
