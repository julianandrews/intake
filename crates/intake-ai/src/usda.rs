use crate::tools::Tool;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde_json::Value;
use std::fmt::Write;
use std::time::Duration;

const SEARCH_URL: &str = "https://api.nal.usda.gov/fdc/v1/foods/search";
const CANDIDATES_PER_QUERY: usize = 5;
const PER_QUERY_CAP: usize = 2000;
const TOTAL_CAP: usize = 2000;

const NUTRIENT_ENERGY_KCAL: i64 = 1008;
const NUTRIENT_PROTEIN: i64 = 1003;
const NUTRIENT_FAT: i64 = 1004;
const NUTRIENT_CARBS: i64 = 1005;
const NUTRIENT_FIBER: i64 = 1079;
const NUTRIENT_FIBER_LEGACY: i64 = 1007;
const NUTRIENT_ALCOHOL: i64 = 1013;

fn round_three(value: f64) -> Option<Decimal> {
    if !value.is_finite() {
        return None;
    }
    let d = Decimal::from_f64(value)?;
    Some(
        d.round_dp_with_strategy(3, RoundingStrategy::MidpointAwayFromZero)
            .normalize(),
    )
}

#[derive(Debug, Clone, Copy, Default)]
struct Per100g {
    calories: Decimal,
    protein_g: Decimal,
    fiber_g: Decimal,
    fat_g: Decimal,
    carbs_g: Decimal,
    alcohol_g: Decimal,
}

impl Per100g {
    fn from_nutrients(nutrients: &Value) -> Per100g {
        let mut p = Per100g::default();
        let Some(arr) = nutrients.as_array() else {
            return p;
        };
        for n in arr {
            let (Some(id), Some(value)) = (n["nutrientId"].as_i64(), n["value"].as_f64()) else {
                continue;
            };
            let Some(v) = round_three(value) else {
                continue;
            };
            match id {
                NUTRIENT_ENERGY_KCAL => p.calories = v,
                NUTRIENT_PROTEIN => p.protein_g = v,
                NUTRIENT_FAT => p.fat_g = v,
                NUTRIENT_CARBS => p.carbs_g = v,
                NUTRIENT_FIBER | NUTRIENT_FIBER_LEGACY => p.fiber_g = v,
                NUTRIENT_ALCOHOL => p.alcohol_g = v,
                _ => {}
            }
        }
        p
    }

    fn macro_list(&self) -> String {
        format!(
            "{} cal, {} protein_g, {} fiber_g, {} fat_g, {} carbs_g, {} alcohol_g",
            self.calories, self.protein_g, self.fiber_g, self.fat_g, self.carbs_g, self.alcohol_g
        )
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(timeout).build()
}

fn no_key_error(tool: &str) -> String {
    format!(
        "{tool}: no USDA API key configured — set `usda_api_key` in the [ai] config or INTAKE_AI_USDA_API_KEY"
    )
}

pub struct UsdaSearch {
    api_key: String,
    agent: ureq::Agent,
    base_url: String,
}

impl UsdaSearch {
    pub fn new(api_key: &str, timeout: Duration) -> UsdaSearch {
        UsdaSearch::with_base(api_key, timeout, SEARCH_URL)
    }

    /// Test hook: point the tool at a local server instead of the USDA API.
    fn with_base(api_key: &str, timeout: Duration, base_url: &str) -> UsdaSearch {
        UsdaSearch {
            api_key: api_key.to_string(),
            agent: agent(timeout),
            base_url: base_url.to_string(),
        }
    }

    fn search_raw(&self, query: &str) -> Result<Value, String> {
        let mut request = self
            .agent
            .get(&self.base_url)
            .query("api_key", &self.api_key)
            .query("query", query)
            .query("pageSize", &CANDIDATES_PER_QUERY.to_string())
            .query("requireAllWords", "false");
        for data_type in ["Foundation", "SR Legacy", "Branded", "Survey (FNDDS)"] {
            request = request.query("dataType", data_type);
        }
        let response = request
            .call()
            .map_err(|e| format!("usda_search failed for query '{query}': {e}"))?;
        let body = response
            .into_string()
            .map_err(|e| format!("usda_search failed to read response for query '{query}': {e}"))?;
        serde_json::from_str(&body)
            .map_err(|e| format!("usda_search: invalid JSON for query '{query}': {e}"))
    }

    fn format_query_with_body(&self, query: &str, body: &Value) -> Result<String, String> {
        let mut out = String::from("query: ");
        out.push_str(query);
        out.push('\n');
        let Some(foods) = body.get("foods").and_then(|f| f.as_array()) else {
            out.push_str("  no results\n");
            return Ok(out);
        };
        if foods.is_empty() {
            out.push_str("  no results\n");
            return Ok(out);
        }
        for (i, food) in foods.iter().enumerate().take(CANDIDATES_PER_QUERY) {
            let Some(fdc_id) = food["fdcId"].as_u64() else {
                continue;
            };
            let Some(description) = food["description"].as_str() else {
                continue;
            };
            let mut parts = vec![fdc_id.to_string(), description.to_string()];
            if let (Some(size), Some(unit)) = (
                food["servingSize"].as_f64(),
                food["servingSizeUnit"].as_str(),
            ) {
                if size > 0.0 {
                    parts.push(format!("{size} {unit}"));
                }
            }
            parts.push(format!(
                "per-100g: {}",
                Per100g::from_nutrients(&food["foodNutrients"]).macro_list()
            ));
            let line = parts.join(" | ");
            if out.chars().count() + line.chars().count() > PER_QUERY_CAP {
                out.push_str("  …\n");
                break;
            }
            let _ = writeln!(out, "  {}. {line}", i + 1);
        }
        Ok(out)
    }

    fn format_query(&self, query: &str) -> Result<String, String> {
        let body = self.search_raw(query)?;
        self.format_query_with_body(query, &body)
    }
}

impl Tool for UsdaSearch {
    fn name(&self) -> &str {
        "usda_search"
    }

    fn description(&self) -> &str {
        "Search the USDA FoodData Central database for foods. Accepts a batch of queries and returns up to five candidate foods per query, each with per-100g macros. Use it to find the right food variant (e.g. raw vs cooked)."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "queries": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "description": "One or more food descriptions to search for."
                }
            },
            "required": ["queries"]
        })
    }

    fn execute(&self, params: &Value) -> Result<String, String> {
        if self.api_key.is_empty() {
            return Err(no_key_error("usda_search"));
        }
        let queries = params["queries"]
            .as_array()
            .ok_or_else(|| "usda_search: missing 'queries' array".to_string())?
            .iter()
            .filter_map(|q| q.as_str())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if queries.is_empty() {
            return Err("usda_search: 'queries' must contain at least one string".to_string());
        }
        let mut out = String::new();
        for (i, query) in queries.iter().enumerate() {
            let block = self.format_query(query)?;
            if out.chars().count() + block.chars().count() > TOTAL_CAP {
                out.push_str("… (output truncated)\n");
                break;
            }
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&block);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Serve one response and capture the raw request line for inspection.
    fn serve_capture(body: Value, captured: Arc<Mutex<String>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            *captured.lock().unwrap() = String::from_utf8_lossy(&buf).into_owned();
            let payload = serde_json::to_string(&body).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                payload.len(),
                payload
            );
            let _ = stream.write_all(response.as_bytes());
        });
        format!("http://{addr}/v1/foods/search")
    }

    /// Serve a single status response (e.g. 429) for one request.
    fn serve_status(status: u16) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let reason = if status == 429 {
                "Too Many Requests"
            } else {
                "Error"
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
            );
            let _ = stream.write_all(response.as_bytes());
        });
        format!("http://{addr}/v1/foods/search")
    }

    fn search_response() -> Value {
        serde_json::json!({
            "foods": [
                {
                    "fdcId": 2347189,
                    "description": "Rice, white, cooked",
                    "servingSize": 158.0,
                    "servingSizeUnit": "g",
                    "foodNutrients": [
                        { "nutrientId": 1008, "value": 130.0 },
                        { "nutrientId": 1003, "value": 2.69 },
                        { "nutrientId": 1004, "value": 0.28 },
                        { "nutrientId": 1005, "value": 28.17 },
                        { "nutrientId": 1079, "value": 0.4 },
                        { "nutrientId": 1013, "value": 0.0 }
                    ]
                },
                {
                    "fdcId": 169757,
                    "description": "Rice, white, raw",
                    "foodNutrients": [
                        { "nutrientId": 1008, "value": 365.0 },
                        { "nutrientId": 1003, "value": 7.13 }
                    ]
                }
            ],
            "totalHits": 2
        })
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str_radix(s, 10).unwrap()
    }

    #[test]
    fn test_round_three() {
        assert_eq!(round_three(2.7).unwrap(), dec("2.7"));
        assert_eq!(round_three(2.6996).unwrap(), dec("2.7"));
        assert_eq!(round_three(f64::NAN), None);
        assert_eq!(round_three(f64::INFINITY), None);
    }

    #[test]
    fn test_per100g_from_nutrients() {
        let p = Per100g::from_nutrients(&search_response()["foods"][0]["foodNutrients"]);
        assert_eq!(p.calories, dec("130"));
        assert_eq!(p.protein_g, dec("2.69"));
        assert_eq!(p.fiber_g, dec("0.4"));
        assert_eq!(p.fat_g, dec("0.28"));
        assert_eq!(p.carbs_g, dec("28.17"));
        assert_eq!(p.alcohol_g, dec("0"));
    }

    #[test]
    fn test_per100g_missing_nutrients_default_zero() {
        let p = Per100g::from_nutrients(&search_response()["foods"][1]["foodNutrients"]);
        assert_eq!(p.calories, dec("365"));
        assert_eq!(p.protein_g, dec("7.13"));
        assert_eq!(p.fiber_g, Decimal::ZERO);
        assert_eq!(p.fat_g, Decimal::ZERO);
        assert_eq!(p.carbs_g, Decimal::ZERO);
        assert_eq!(p.alcohol_g, Decimal::ZERO);
    }

    #[test]
    fn test_legacy_fiber_nutrient_id() {
        let nutrients = serde_json::json!([{ "nutrientId": 1007, "value": 2.5 }]);
        let p = Per100g::from_nutrients(&nutrients);
        assert_eq!(p.fiber_g, dec("2.5"));
    }

    #[test]
    fn test_usda_search_formats_candidates() {
        let tool = UsdaSearch::new("key", Duration::from_secs(5));
        let out = tool
            .format_query_with_body("rice", &search_response())
            .unwrap();
        assert!(out.starts_with("query: rice\n"));
        assert!(out.contains("1. 2347189 | Rice, white, cooked | 158 g | per-100g: 130 cal, 2.69 protein_g, 0.4 fiber_g, 0.28 fat_g, 28.17 carbs_g, 0 alcohol_g"));
        assert!(out.contains("2. 169757 | Rice, white, raw | per-100g: 365 cal, 7.13 protein_g, 0 fiber_g, 0 fat_g, 0 carbs_g, 0 alcohol_g"));
    }

    #[test]
    fn test_usda_search_no_results() {
        let tool = UsdaSearch::new("key", Duration::from_secs(5));
        let out = tool
            .format_query_with_body("zzz", &serde_json::json!({ "foods": [] }))
            .unwrap();
        assert!(out.contains("no results"));
    }

    #[test]
    fn test_usda_search_missing_key_errors() {
        let tool = UsdaSearch::new("", Duration::from_secs(5));
        let err = tool
            .execute(&serde_json::json!({ "queries": ["rice"] }))
            .unwrap_err();
        assert!(err.contains("API key"));
    }

    #[test]
    fn test_usda_search_requires_queries() {
        let tool = UsdaSearch::new("key", Duration::from_secs(5));
        assert!(tool.execute(&serde_json::json!({})).is_err());
        assert!(tool.execute(&serde_json::json!({ "queries": [] })).is_err());
    }

    #[test]
    fn test_usda_tool_schemas() {
        let search = UsdaSearch::new("key", Duration::from_secs(5));
        assert_eq!(search.name(), "usda_search");
        assert!(search.schema()["required"].as_array().is_some());
    }

    #[test]
    fn test_macro_list_order() {
        let p = Per100g::from_nutrients(&search_response()["foods"][0]["foodNutrients"]);
        assert_eq!(
            p.macro_list(),
            "130 cal, 2.69 protein_g, 0.4 fiber_g, 0.28 fat_g, 28.17 carbs_g, 0 alcohol_g"
        );
    }

    #[test]
    fn test_usda_search_request_sends_all_four_data_types() {
        let captured = Arc::new(Mutex::new(String::new()));
        let base = serve_capture(search_response(), Arc::clone(&captured));
        let tool = UsdaSearch::with_base("key", Duration::from_secs(5), &base);
        let out = tool
            .execute(&serde_json::json!({ "queries": ["rice"] }))
            .unwrap();
        assert!(out.contains("Rice, white, cooked"));
        let request = captured.lock().unwrap().clone();
        assert!(
            request.starts_with("GET /v1/foods/search?"),
            "got: {request}"
        );
        assert!(request.contains("api_key=key"), "got: {request}");
        assert!(request.contains("query=rice"), "got: {request}");
        assert_eq!(request.matches("dataType=").count(), 4, "got: {request}");
        for dt in [
            "dataType=Foundation",
            "dataType=SR+Legacy",
            "dataType=Branded",
            "dataType=Survey+%28FNDDS%29",
        ] {
            assert!(request.contains(dt), "missing {dt}: {request}");
        }
        assert!(
            !request.contains("dataType=FNDDS"),
            "invalid enum value 'FNDDS' matches nothing — must be 'Survey (FNDDS)': {request}"
        );
    }

    #[test]
    fn test_usda_search_rate_limit_429_returns_error_string() {
        let base = serve_status(429);
        let tool = UsdaSearch::with_base("key", Duration::from_secs(5), &base);
        let err = tool
            .execute(&serde_json::json!({ "queries": ["rice"] }))
            .unwrap_err();
        assert!(err.contains("usda_search failed"), "got: {err}");
        assert!(err.contains("429"), "got: {err}");
    }

    #[test]
    fn test_usda_search_fetch_failure_returns_error_string() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let base = format!("http://{addr}/v1/foods/search");
        let tool = UsdaSearch::with_base("key", Duration::from_secs(1), &base);
        let err = tool
            .execute(&serde_json::json!({ "queries": ["rice"] }))
            .unwrap_err();
        assert!(err.contains("usda_search failed"), "got: {err}");
    }
}
