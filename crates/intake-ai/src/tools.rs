pub trait Tool {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    fn execute(&self, params: &serde_json::Value) -> Result<String, String>;
}

pub fn function_definitions(tools: &[&dyn Tool]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.schema(),
                }
            })
        })
        .collect()
}
