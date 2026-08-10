use serde::Deserialize;

pub const DEFAULT_MODEL: &str = "gpt-4o-mini";
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_MAX_RETRIES: u32 = 3;
pub const DEFAULT_MAX_TOOL_CALLS: u32 = 20;
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
pub const DEFAULT_USDA_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiSettings {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: u32,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub usda_api_key: Option<String>,
    #[serde(default = "default_usda_timeout_secs")]
    pub usda_timeout_secs: u64,
    #[serde(default)]
    pub trace_requests: bool,
    #[serde(default)]
    pub trace_responses: bool,
    #[serde(default)]
    pub trace_colors: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            api_key: String::new(),
            model: default_model(),
            base_url: default_base_url(),
            max_retries: default_max_retries(),
            max_tool_calls: default_max_tool_calls(),
            timeout_secs: default_timeout_secs(),
            usda_api_key: None,
            usda_timeout_secs: default_usda_timeout_secs(),
            trace_requests: false,
            trace_responses: false,
            trace_colors: false,
        }
    }
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.to_string()
}

fn default_max_retries() -> u32 {
    DEFAULT_MAX_RETRIES
}

fn default_max_tool_calls() -> u32 {
    DEFAULT_MAX_TOOL_CALLS
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

fn default_usda_timeout_secs() -> u64 {
    DEFAULT_USDA_TIMEOUT_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let s = AiSettings::default();
        assert_eq!(s.model, DEFAULT_MODEL);
        assert_eq!(s.base_url, DEFAULT_BASE_URL);
        assert_eq!(s.max_retries, 3);
        assert_eq!(s.max_tool_calls, 20);
        assert_eq!(s.timeout_secs, 60);
        assert_eq!(s.usda_timeout_secs, 15);
        assert!(!s.trace_requests);
        assert!(!s.trace_responses);
        assert!(!s.trace_colors);
        assert_eq!(s.usda_api_key, None);
    }

    #[test]
    fn test_deserialize_with_defaults() {
        let s: AiSettings = toml::from_str("").unwrap();
        assert_eq!(s.model, DEFAULT_MODEL);
        assert_eq!(s.base_url, DEFAULT_BASE_URL);
        assert_eq!(s.max_retries, 3);
        assert!(!s.trace_requests);
        assert!(!s.trace_responses);
        assert!(!s.trace_colors);
    }

    #[test]
    fn test_deserialize_override() {
        let s: AiSettings = toml::from_str(
            "api_key = \"k\"\nmodel = \"deepseek\"\nbase_url = \"http://localhost:11434/v1\"\nmax_retries = 5\nusda_api_key = \"u\"\ntrace_requests = true\ntrace_responses = true\ntrace_colors = true\n",
        )
        .unwrap();
        assert_eq!(s.api_key, "k");
        assert_eq!(s.model, "deepseek");
        assert_eq!(s.base_url, "http://localhost:11434/v1");
        assert_eq!(s.max_retries, 5);
        assert_eq!(s.usda_api_key.as_deref(), Some("u"));
        assert!(s.trace_requests);
        assert!(s.trace_responses);
        assert!(s.trace_colors);
    }

    #[test]
    fn test_trace_toggles_independent() {
        let s: AiSettings = toml::from_str("trace_requests = true\n").unwrap();
        assert!(s.trace_requests);
        assert!(!s.trace_responses);
        let s: AiSettings = toml::from_str("trace_responses = true\n").unwrap();
        assert!(!s.trace_requests);
        assert!(s.trace_responses);
        let s: AiSettings = toml::from_str("trace_colors = true\n").unwrap();
        assert!(s.trace_colors);
        assert!(!s.trace_requests);
    }

    #[test]
    fn test_unknown_field_rejected() {
        assert!(toml::from_str::<AiSettings>("bogus = 1\n").is_err());
    }
}
