use serde::de::Error as DeError;
use serde::Deserialize;

/// Default timeout for the USDA search tool when `usda_timeout_secs` is
/// unset.
pub const DEFAULT_USDA_TIMEOUT_SECS: u64 = 15;

/// The `[ai]` config table: every key is optional, spanning the generic
/// `intake-ai` settings fields plus the intake-owned keys (`usda_api_key`,
/// `usda_timeout_secs`, `history_days`, prompt overrides). Missing values
/// are filled later, when the final `intake_ai::Settings` is resolved.
/// Deserialization rejects unknown keys with a friendly message instead of
/// the default serde one.
#[derive(Debug)]
pub(crate) struct AiConfig {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub max_retries: Option<u32>,
    pub max_tool_calls: Option<u32>,
    pub timeout_secs: Option<u64>,
    pub trace_requests: Option<bool>,
    pub trace_responses: Option<bool>,
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
#[serde(deny_unknown_fields)]
struct RawAiConfig {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    max_retries: Option<u32>,
    #[serde(default)]
    max_tool_calls: Option<u32>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    trace_requests: Option<bool>,
    #[serde(default)]
    trace_responses: Option<bool>,
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
            api_key: raw.api_key,
            model: raw.model,
            base_url: raw.base_url,
            max_retries: raw.max_retries,
            max_tool_calls: raw.max_tool_calls,
            timeout_secs: raw.timeout_secs,
            trace_requests: raw.trace_requests,
            trace_responses: raw.trace_responses,
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
        assert_eq!(ai.api_key.as_deref(), Some("k"));
        assert_eq!(ai.model.as_deref(), Some("m"));
        assert_eq!(ai.max_retries, None);
        assert_eq!(ai.history_days, Some(7));
        assert_eq!(ai.log_prompt.as_deref(), Some("x"));
        assert_eq!(ai.food_new_prompt, None);
    }

    #[test]
    fn test_ai_config_empty_table_all_none() {
        let config: Config = toml::from_str("[ai]\n").unwrap();
        let ai = config.ai.unwrap();
        assert_eq!(ai.model, None);
        assert_eq!(ai.base_url, None);
        assert_eq!(ai.history_days, None);
        assert_eq!(ai.log_prompt, None);
    }

    #[test]
    fn test_ai_config_unknown_key_rejected() {
        assert!(toml::from_str::<Config>("[ai]\nbogus = 1\n").is_err());
    }

    #[test]
    fn test_ai_config_single_field_only() {
        let config: Config = toml::from_str("[ai]\nmax_retries = 7\n").unwrap();
        let ai = config.ai.unwrap();
        assert_eq!(ai.max_retries, Some(7));
        assert_eq!(ai.model, None);
        assert_eq!(ai.base_url, None);
        assert_eq!(ai.max_tool_calls, None);
        assert_eq!(ai.timeout_secs, None);
    }

    #[test]
    fn test_ai_config_trace_toggles_independent() {
        let config: Config = toml::from_str("[ai]\ntrace_requests = true\n").unwrap();
        let ai = config.ai.unwrap();
        assert_eq!(ai.trace_requests, Some(true));
        assert_eq!(ai.trace_responses, None);
        let config: Config = toml::from_str("[ai]\ntrace_responses = true\n").unwrap();
        let ai = config.ai.unwrap();
        assert_eq!(ai.trace_requests, None);
        assert_eq!(ai.trace_responses, Some(true));
    }

    #[test]
    fn test_ai_config_trace_colors_rejected() {
        let err = toml::from_str::<Config>("[ai]\ntrace_colors = true\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("trace_colors"), "got: {err}");
    }
}
