use anyhow::{bail, Context};
use serde::de::Error as DeError;
use serde::Deserialize;
use std::path::PathBuf;

/// Default timeout for the USDA search tool when `usda_timeout_secs` is
/// unset.
pub const DEFAULT_USDA_TIMEOUT_SECS: u64 = 15;

/// A configured secret. TOML may give it inline as a string, or as a table
/// with exactly one `file` or `command` key:
///
/// ```toml
/// api_key = "inline"
/// api_key = { file = "/path/to/key" }
/// api_key = { command = "op read op://private/openai/key" }
/// ```
///
/// `file` is read as UTF-8 and trimmed; `command` runs through the platform
/// shell (`sh -c` on Unix, `cmd /C` on Windows) and its trimmed stdout is the
/// value. Resolution happens only when the config value is actually selected,
/// so a flag or non-empty environment variable skips running the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Secret {
    Inline(String),
    File(PathBuf),
    Command(String),
}

impl Secret {
    /// Resolve the source to its secret value. `label` names the setting in
    /// error messages (e.g. `api_key`) and never includes the value itself.
    /// Relative `file` paths resolve against `base_dir` (the config file's
    /// directory) when one is known.
    pub(crate) fn resolve(
        &self,
        label: &str,
        base_dir: Option<&std::path::Path>,
    ) -> anyhow::Result<String> {
        let value = match self {
            Secret::Inline(value) => value.clone(),
            Secret::File(path) => {
                let path = if path.is_absolute() {
                    path.clone()
                } else if let Some(base) = base_dir {
                    base.join(path)
                } else {
                    path.clone()
                };
                std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {label} file {}", path.display()))?
                    .trim()
                    .to_string()
            }
            Secret::Command(command) => {
                let (shell, flag) = if cfg!(windows) {
                    ("cmd", "/C")
                } else {
                    ("sh", "-c")
                };
                let output = std::process::Command::new(shell)
                    .arg(flag)
                    .arg(command)
                    .output()
                    .with_context(|| format!("failed to run {label} command"))?;
                if !output.status.success() {
                    bail!("{label} command failed ({})", output.status);
                }
                String::from_utf8(output.stdout)
                    .with_context(|| format!("{label} command produced non-UTF-8 output"))?
                    .trim()
                    .to_string()
            }
        };
        match self {
            // Inline is literal: preserve it exactly, including an empty
            // string (which keeps the old "no auth" behavior).
            Secret::Inline(_) => Ok(value),
            Secret::File(_) | Secret::Command(_) => {
                if value.is_empty() {
                    bail!("{label} resolved to an empty value");
                }
                Ok(value)
            }
        }
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SecretVisitor;

        impl<'de> serde::de::Visitor<'de> for SecretVisitor {
            type Value = Secret;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string, or a table with exactly one `file` or `command` key")
            }

            fn visit_str<E>(self, value: &str) -> Result<Secret, E>
            where
                E: serde::de::Error,
            {
                Ok(Secret::Inline(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Secret, E>
            where
                E: serde::de::Error,
            {
                Ok(Secret::Inline(value))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Secret, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut file: Option<PathBuf> = None;
                let mut command: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "file" => {
                            if file.is_some() {
                                return Err(serde::de::Error::duplicate_field("file"));
                            }
                            file = Some(map.next_value()?);
                        }
                        "command" => {
                            if command.is_some() {
                                return Err(serde::de::Error::duplicate_field("command"));
                            }
                            command = Some(map.next_value()?);
                        }
                        other => {
                            return Err(serde::de::Error::unknown_field(
                                other,
                                &["file", "command"],
                            ));
                        }
                    }
                }
                match (file, command) {
                    (Some(path), None) => Ok(Secret::File(path)),
                    (None, Some(command)) => Ok(Secret::Command(command)),
                    (Some(_), Some(_)) => Err(serde::de::Error::custom(
                        "secret must set either `file` or `command`, not both",
                    )),
                    (None, None) => Err(serde::de::Error::custom(
                        "secret table must set either `file` or `command`",
                    )),
                }
            }
        }

        deserializer.deserialize_any(SecretVisitor)
    }
}

/// The `[ai]` config table: every key is optional, spanning the generic
/// `intake-ai` settings fields plus the intake-owned keys (`usda_api_key`,
/// `usda_timeout_secs`, `history_days`, prompt overrides). Missing values
/// are filled later, when the final `intake_ai::Settings` is resolved.
/// Deserialization rejects unknown keys with a friendly message instead of
/// the default serde one.
#[derive(Debug)]
pub(crate) struct AiConfig {
    pub api_key: Option<Secret>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub max_retries: Option<u32>,
    pub max_tool_calls: Option<u32>,
    pub timeout_secs: Option<u64>,
    pub trace_requests: Option<bool>,
    pub trace_responses: Option<bool>,
    pub session_header: Option<String>,
    pub usda_api_key: Option<Secret>,
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
    "session_header",
    "history_days",
    "log_prompt",
    "food_new_prompt",
    "food_edit_prompt",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAiConfig {
    #[serde(default)]
    api_key: Option<Secret>,
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
    session_header: Option<String>,
    #[serde(default)]
    usda_api_key: Option<Secret>,
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
            session_header: raw.session_header,
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
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_ai_config_parses() {
        let config: Config = toml::from_str(
            "[ai]\napi_key = \"k\"\nmodel = \"m\"\nhistory_days = 7\nlog_prompt = \"x\"\n",
        )
        .unwrap();
        let ai = config.ai.unwrap();
        assert_eq!(ai.api_key, Some(Secret::Inline("k".to_string())));
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
    fn test_ai_config_session_header_parses() {
        let config: Config =
            toml::from_str("[ai]\nsession_header = \"x-opencode-session\"\n").unwrap();
        let ai = config.ai.unwrap();
        assert_eq!(ai.session_header.as_deref(), Some("x-opencode-session"));
        assert_eq!(ai.api_key, None);
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

    #[test]
    fn test_ai_config_api_key_sources_parse() {
        let config: Config = toml::from_str("[ai]\napi_key = { file = \"/tmp/key\" }\n").unwrap();
        assert_eq!(
            config.ai.unwrap().api_key,
            Some(Secret::File(PathBuf::from("/tmp/key")))
        );

        let config: Config =
            toml::from_str("[ai]\napi_key = { command = \"op read op://x/y\" }\n").unwrap();
        assert_eq!(
            config.ai.unwrap().api_key,
            Some(Secret::Command("op read op://x/y".to_string()))
        );
    }

    #[test]
    fn test_ai_config_usda_key_sources_parse() {
        let config: Config =
            toml::from_str("[ai]\nusda_api_key = { file = \"/tmp/usda\" }\n").unwrap();
        assert_eq!(
            config.ai.unwrap().usda_api_key,
            Some(Secret::File(PathBuf::from("/tmp/usda")))
        );
    }

    #[test]
    fn test_ai_config_secret_rejects_both_sources() {
        let err = toml::from_str::<Config>(
            "[ai]\napi_key = { file = \"/tmp/key\", command = \"echo k\" }\n",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("either `file` or `command`"), "got: {err}");
    }

    #[test]
    fn test_ai_config_secret_rejects_empty_table() {
        let err = toml::from_str::<Config>("[ai]\napi_key = {}\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("either `file` or `command`"), "got: {err}");
    }

    #[test]
    fn test_ai_config_secret_rejects_unknown_source() {
        let err = toml::from_str::<Config>("[ai]\napi_key = { env = \"KEY\" }\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field"), "got: {err}");
        assert!(err.contains("env"), "got: {err}");
    }

    #[test]
    fn test_secret_inline_resolves_as_written() {
        assert_eq!(
            Secret::Inline("  key  ".to_string())
                .resolve("api_key", None)
                .unwrap(),
            "  key  "
        );
        assert_eq!(
            Secret::Inline(String::new())
                .resolve("api_key", None)
                .unwrap(),
            ""
        );
    }

    #[test]
    fn test_secret_file_resolves_trimmed() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "  file-key\n").unwrap();
        assert_eq!(
            Secret::File(path).resolve("api_key", None).unwrap(),
            "file-key"
        );
    }

    #[test]
    fn test_secret_file_relative_resolves_against_base_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("key"), "relative-key\n").unwrap();
        assert_eq!(
            Secret::File(PathBuf::from("key"))
                .resolve("api_key", Some(dir.path()))
                .unwrap(),
            "relative-key"
        );
    }

    #[test]
    fn test_secret_file_missing_errors_with_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("missing");
        let err = Secret::File(path.clone())
            .resolve("api_key", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("failed to read api_key file"), "got: {err}");
        assert!(err.contains(&path.display().to_string()), "got: {err}");
    }

    #[test]
    fn test_secret_file_empty_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("empty");
        std::fs::write(&path, " \n").unwrap();
        let err = Secret::File(path)
            .resolve("api_key", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("empty"), "got: {err}");
    }

    #[test]
    fn test_secret_command_resolves_trimmed_stdout() {
        let secret = Secret::Command("echo command-key".to_string());
        assert_eq!(secret.resolve("api_key", None).unwrap(), "command-key");
    }

    #[test]
    fn test_secret_command_failure_errors() {
        let err = Secret::Command("exit 1".to_string())
            .resolve("api_key", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("api_key command failed"), "got: {err}");
        assert!(!err.contains("exit 1"), "must not echo the command: {err}");
    }

    #[test]
    fn test_secret_command_empty_output_errors() {
        let command = if cfg!(windows) { "rem" } else { "true" };
        let err = Secret::Command(command.to_string())
            .resolve("api_key", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("empty"), "got: {err}");
    }
}
