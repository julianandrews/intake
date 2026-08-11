use serde::de::Error as DeError;
use serde::Deserialize;

/// Default timeout for the USDA search tool when `usda_timeout_secs` is
/// unset.
pub const DEFAULT_USDA_TIMEOUT_SECS: u64 = 15;

/// The `[ai]` config table: the generic `intake-ai` settings plus the
/// intake-owned keys (`usda_api_key`, `usda_timeout_secs`, `history_days`,
/// prompt overrides). Deserialization rejects unknown keys with a friendly
/// message instead of the default serde one.
#[derive(Debug)]
pub(crate) struct AiConfig {
    pub settings: intake_ai::settings::AiSettings,
    pub usda_api_key: Option<String>,
    pub usda_timeout_secs: Option<u64>,
    pub history_days: Option<u32>,
    pub log_prompt: Option<String>,
    pub food_new_prompt: Option<String>,
    pub food_edit_prompt: Option<String>,
}

const AI_CONFIG_KEYS: &[&str] = &[
    "api_key",
    "model",
    "base_url",
    "max_retries",
    "max_tool_calls",
    "timeout_secs",
    "usda_api_key",
    "usda_timeout_secs",
    "trace_requests",
    "trace_responses",
    "history_days",
    "log_prompt",
    "food_new_prompt",
    "food_edit_prompt",
];

#[derive(Deserialize)]
struct RawAiConfig {
    #[serde(flatten)]
    settings: intake_ai::settings::AiSettings,
    #[serde(default)]
    usda_api_key: Option<String>,
    #[serde(default)]
    usda_timeout_secs: Option<u64>,
    history_days: Option<u32>,
    log_prompt: Option<String>,
    food_new_prompt: Option<String>,
    food_edit_prompt: Option<String>,
}

impl<'de> Deserialize<'de> for AiConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let map = value
            .as_object()
            .ok_or_else(|| D::Error::custom("expected a table for `[ai]`"))?;
        for key in map.keys() {
            if !AI_CONFIG_KEYS.contains(&key.as_str()) {
                return Err(D::Error::custom(format!("unknown field `{key}` in `[ai]`")));
            }
        }
        let raw = RawAiConfig::deserialize(&value).map_err(D::Error::custom)?;
        Ok(AiConfig {
            settings: raw.settings,
            usda_api_key: raw.usda_api_key,
            usda_timeout_secs: raw.usda_timeout_secs,
            history_days: raw.history_days,
            log_prompt: raw.log_prompt,
            food_new_prompt: raw.food_new_prompt,
            food_edit_prompt: raw.food_edit_prompt,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn test_ai_config_parses() {
        let config: Config = toml::from_str(
            "[ai]\napi_key = \"k\"\nmodel = \"m\"\nhistory_days = 7\nlog_prompt = \"x\"\n",
        )
        .unwrap();
        let ai = config.ai.unwrap();
        assert_eq!(ai.settings.api_key, "k");
        assert_eq!(ai.settings.model, "m");
        assert_eq!(ai.settings.max_retries, 3);
        assert_eq!(ai.history_days, Some(7));
        assert_eq!(ai.log_prompt.as_deref(), Some("x"));
        assert_eq!(ai.food_new_prompt, None);
    }

    #[test]
    fn test_ai_config_empty_table_uses_defaults() {
        let config: Config = toml::from_str("[ai]\n").unwrap();
        let ai = config.ai.unwrap();
        assert_eq!(ai.settings.model, intake_ai::settings::DEFAULT_MODEL);
        assert_eq!(ai.settings.base_url, intake_ai::settings::DEFAULT_BASE_URL);
        assert_eq!(ai.history_days, None);
    }

    #[test]
    fn test_ai_config_unknown_key_rejected() {
        assert!(toml::from_str::<Config>("[ai]\nbogus = 1\n").is_err());
    }

    #[test]
    fn test_ai_config_trace_colors_rejected() {
        let err = toml::from_str::<Config>("[ai]\ntrace_colors = true\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("trace_colors"), "got: {err}");
    }
}
