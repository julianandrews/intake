pub const DEFAULT_MAX_RETRIES: u32 = 3;
pub const DEFAULT_MAX_TOOL_CALLS: u32 = 20;
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone)]
pub struct Settings {
    pub api_key: Option<String>,
    pub model: String,
    pub base_url: String,
    pub max_retries: u32,
    pub max_tool_calls: u32,
    pub timeout_secs: u64,
    pub trace_requests: bool,
    pub trace_responses: bool,
}

impl Settings {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Settings {
        Settings {
            api_key,
            model: model.into(),
            base_url: base_url.into(),
            max_retries: DEFAULT_MAX_RETRIES,
            max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            trace_requests: false,
            trace_responses: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_fills_operational_defaults() {
        let s = Settings::new("http://localhost:11434/v1", "llama3", Some("k".to_string()));
        assert_eq!(s.base_url, "http://localhost:11434/v1");
        assert_eq!(s.model, "llama3");
        assert_eq!(s.api_key.as_deref(), Some("k"));
        assert_eq!(s.max_retries, DEFAULT_MAX_RETRIES);
        assert_eq!(s.max_tool_calls, DEFAULT_MAX_TOOL_CALLS);
        assert_eq!(s.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert!(!s.trace_requests);
        assert!(!s.trace_responses);
    }

    #[test]
    fn test_new_api_key_optional() {
        let s = Settings::new("http://x", "m", None);
        assert_eq!(s.api_key, None);
    }
}
