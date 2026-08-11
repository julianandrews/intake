pub trait Tool {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    fn execute(&self, params: &serde_json::Value) -> Result<String, String>;

    /// This tool's OpenAI function-definition envelope, as it appears in the
    /// request body's `tools` array.
    fn to_api_json(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": self.schema(),
            }
        })
    }
}
