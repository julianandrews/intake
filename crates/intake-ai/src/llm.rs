use crate::settings::AiSettings;
use crate::tools::{function_definitions, Tool};
use serde_json::Value;
use std::fmt;
use std::io::Write;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssistantMessage {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub reasoning: Option<String>,
}

impl AssistantMessage {
    pub fn text(content: &str) -> AssistantMessage {
        AssistantMessage {
            content: Some(content.to_string()),
            tool_calls: Vec::new(),
            reasoning: None,
        }
    }

    pub fn with_tools(content: Option<&str>, tool_calls: Vec<ToolCall>) -> AssistantMessage {
        AssistantMessage {
            content: content.map(str::to_string),
            tool_calls,
            reasoning: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    System(String),
    User(String),
    Assistant(AssistantMessage),
    Tool { call_id: String, content: String },
}

impl Message {
    fn to_api_json(&self) -> Value {
        match self {
            Message::System(content) => serde_json::json!({ "role": "system", "content": content }),
            Message::User(content) => serde_json::json!({ "role": "user", "content": content }),
            Message::Tool { call_id, content } => {
                serde_json::json!({ "role": "tool", "tool_call_id": call_id, "content": content })
            }
            Message::Assistant(msg) => {
                let mut m = serde_json::json!({ "role": "assistant", "content": msg.content });
                if !msg.tool_calls.is_empty() {
                    m["tool_calls"] = Value::Array(
                        msg.tool_calls
                            .iter()
                            .map(|c| {
                                serde_json::json!({
                                    "id": c.id,
                                    "type": "function",
                                    "function": { "name": c.name, "arguments": c.arguments },
                                })
                            })
                            .collect(),
                    );
                }
                m
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmError {
    Timeout,
    Http { status: u16, body: String },
    BadResponse(String),
    Transport(String),
}

impl LlmError {
    pub fn message(&self) -> String {
        match self {
            LlmError::Timeout => "LLM request timed out".to_string(),
            LlmError::Http { status, body } => {
                let mut m = format!("LLM request failed with HTTP {status}: {body}");
                if body.to_ascii_lowercase().contains("tools") {
                    m.push_str(
                        " — the model/provider appears to reject function calling (tools); use a model that supports tool calling",
                    );
                }
                m
            }
            LlmError::BadResponse(detail) => format!("invalid LLM response: {detail}"),
            LlmError::Transport(detail) => format!("LLM request failed: {detail}"),
        }
    }
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

pub trait LlmBackend {
    fn complete(
        &self,
        messages: &[Message],
        tools: &[&dyn Tool],
    ) -> Result<AssistantMessage, LlmError>;
}

pub struct OpenAiCompatible {
    settings: AiSettings,
    agent: ureq::Agent,
    backoff: Duration,
}

/// Total attempts per API call on retryable HTTP statuses (429, 5xx). Fixed
/// by design, separate from `AiSettings::max_retries`, which covers only
/// output-validation retries in the resolve loop (see the design doc).
const MAX_HTTP_ATTEMPTS: u32 = 3;

impl OpenAiCompatible {
    pub fn new(settings: &AiSettings) -> OpenAiCompatible {
        OpenAiCompatible::with_tuning(settings, Duration::from_millis(500), None)
    }

    pub fn with_tuning(
        settings: &AiSettings,
        backoff: Duration,
        timeout: Option<Duration>,
    ) -> OpenAiCompatible {
        let timeout = timeout.unwrap_or_else(|| Duration::from_secs(settings.timeout_secs));
        let agent = ureq::AgentBuilder::new().timeout(timeout).build();
        OpenAiCompatible {
            settings: settings.clone(),
            agent,
            backoff,
        }
    }

    fn request_body(&self, messages: &[Message], tools: &[&dyn Tool]) -> Value {
        let mut body = serde_json::json!({
            "model": self.settings.model,
            "messages": messages.iter().map(Message::to_api_json).collect::<Vec<_>>(),
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(function_definitions(tools));
        }
        body
    }

    fn send_once(&self, messages: &[Message], tools: &[&dyn Tool]) -> Result<String, LlmError> {
        let url = format!(
            "{}/chat/completions",
            self.settings.base_url.trim_end_matches('/')
        );
        let body = self.request_body(messages, tools);

        let mut request = self.agent.post(&url);
        if !self.settings.api_key.is_empty() {
            request = request.set(
                "Authorization",
                &format!("Bearer {}", self.settings.api_key),
            );
        }

        match request.send_json(body) {
            Ok(response) => response
                .into_string()
                .map_err(|e| LlmError::Transport(format!("failed to read response: {e}"))),
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_else(|_| String::new());
                Err(LlmError::Http { status, body })
            }
            Err(ureq::Error::Transport(t)) => {
                let detail = t.to_string();
                if detail.to_ascii_lowercase().contains("timed out") {
                    Err(LlmError::Timeout)
                } else {
                    Err(LlmError::Transport(detail))
                }
            }
        }
    }

    fn complete_with_retry(
        &self,
        messages: &[Message],
        tools: &[&dyn Tool],
    ) -> Result<String, LlmError> {
        let mut attempts: u32 = 0;
        loop {
            match self.send_once(messages, tools) {
                Ok(raw) => return Ok(raw),
                Err(LlmError::Http { status, body })
                    if status == 429 || (500..=599).contains(&status) =>
                {
                    attempts += 1;
                    if attempts >= MAX_HTTP_ATTEMPTS {
                        return Err(LlmError::Http { status, body });
                    }
                    std::thread::sleep(self.backoff * attempts);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl LlmBackend for OpenAiCompatible {
    fn complete(
        &self,
        messages: &[Message],
        tools: &[&dyn Tool],
    ) -> Result<AssistantMessage, LlmError> {
        let raw = self.complete_with_retry(messages, tools)?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|e| LlmError::BadResponse(format!("invalid JSON: {e}")))?;
        parse_response(&value)
    }
}

fn parse_tool_call(value: &Value) -> Option<ToolCall> {
    let id = value.get("id")?.as_str()?.to_string();
    let name = value["function"]["name"].as_str()?.to_string();
    let arguments = value["function"]["arguments"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_default();
    Some(ToolCall {
        id,
        name,
        arguments,
    })
}

fn parse_response(raw: &Value) -> Result<AssistantMessage, LlmError> {
    let message = raw
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .ok_or_else(|| LlmError::BadResponse("missing choices[0].message".to_string()))?;
    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .map(str::to_string);
    let reasoning = message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .and_then(|r| r.as_str())
        .map(str::to_string);
    let tool_calls = message
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(parse_tool_call).collect())
        .unwrap_or_default();
    Ok(AssistantMessage {
        content,
        tool_calls,
        reasoning,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentOutcome {
    Final(AssistantMessage),
    Exhausted(AssistantMessage),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Trace {
    pub requests: bool,
    pub responses: bool,
    pub colors: bool,
    printed: usize,
}

impl Trace {
    pub fn new(requests: bool, responses: bool, colors: bool) -> Trace {
        Trace {
            requests,
            responses,
            colors,
            printed: 0,
        }
    }

    fn new_messages<'a>(&mut self, messages: &'a [Message]) -> &'a [Message] {
        let start = self.printed.min(messages.len());
        self.printed = messages.len();
        &messages[start..]
    }
}

fn execute_tool(tools: &[&dyn Tool], call: &ToolCall) -> String {
    let Some(tool) = tools.iter().find(|t| t.name() == call.name) else {
        return format!("error: no such tool '{}'", call.name);
    };
    let params: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
    match tool.execute(&params) {
        Ok(out) => out,
        Err(e) => format!("error: {e}"),
    }
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_GREEN: &str = "\x1b[32m";
pub(crate) const ANSI_RED: &str = "\x1b[31m";

pub(crate) fn paint(enabled: bool, text: &str, ansi: &str) -> String {
    if enabled {
        format!("{ansi}{text}{ANSI_RESET}")
    } else {
        text.to_string()
    }
}

fn marker(enabled: bool, title: &str) -> String {
    paint(enabled, title, ANSI_BOLD_YELLOW)
}

fn trace_message(msg: &AssistantMessage, trace: Trace) {
    write_trace_message(&mut std::io::stderr(), msg, trace)
}

fn write_trace_message(out: &mut impl Write, msg: &AssistantMessage, trace: Trace) {
    if !trace.responses {
        return;
    }
    writeln!(out, "{}", marker(trace.colors, "--- from model ---")).unwrap();
    if let Some(reasoning) = &msg.reasoning {
        writeln!(
            out,
            "{}",
            paint(
                trace.colors,
                &format!("[reasoning] {reasoning}"),
                ANSI_GREEN
            )
        )
        .unwrap();
    }
    for call in &msg.tool_calls {
        writeln!(
            out,
            "{}",
            paint(
                trace.colors,
                &format!("[tool] {} {}", call.name, call.arguments),
                ANSI_GREEN
            )
        )
        .unwrap();
    }
    if let Some(content) = &msg.content {
        writeln!(out, "{}", paint(trace.colors, content, ANSI_GREEN)).unwrap();
    }
    writeln!(out, "{}", marker(trace.colors, "--- end from model ---")).unwrap();
    writeln!(out).unwrap();
}

fn trace_messages(messages: &[Message], trace: Trace) {
    write_trace_messages(&mut std::io::stderr(), messages, trace)
}

fn write_trace_messages(out: &mut impl Write, messages: &[Message], trace: Trace) {
    if !trace.requests {
        return;
    }
    writeln!(out, "{}", marker(trace.colors, "--- to model ---")).unwrap();
    for message in messages {
        let line = match message {
            Message::System(content) => format!("[system] {content}"),
            Message::User(content) => format!("[user] {content}"),
            Message::Assistant(msg) => match &msg.content {
                Some(content) => format!("[assistant] {content}"),
                None => format!(
                    "[assistant] (tool calls: {})",
                    msg.tool_calls
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            },
            Message::Tool { call_id, content } => format!("[tool:{call_id}] {content}"),
        };
        writeln!(out, "{}", paint(trace.colors, &line, ANSI_CYAN)).unwrap();
    }
    writeln!(out, "{}", marker(trace.colors, "--- end to model ---")).unwrap();
    writeln!(out).unwrap();
}

pub fn run_agent_loop(
    backend: &dyn LlmBackend,
    messages: &mut Vec<Message>,
    tools: &[&dyn Tool],
    max_tool_calls: u32,
    trace: &mut Trace,
) -> Result<AgentOutcome, LlmError> {
    let mut executions: u32 = 0;
    loop {
        let new = trace.new_messages(messages);
        trace_messages(new, *trace);
        let msg = backend.complete(messages, tools)?;
        trace_message(&msg, *trace);
        if msg.tool_calls.is_empty() {
            messages.push(Message::Assistant(msg.clone()));
            return Ok(AgentOutcome::Final(msg));
        }

        let mut exhausted = executions >= max_tool_calls;
        if !exhausted {
            messages.push(Message::Assistant(msg.clone()));
            for call in &msg.tool_calls {
                if executions >= max_tool_calls {
                    exhausted = true;
                }
                // Every tool call in the assistant message must be answered,
                // or the conversation is invalid for the API: calls beyond
                // the budget get a skipped result instead of being dropped.
                let result = if exhausted {
                    format!(
                        "skipped: tool call budget exhausted (max {max_tool_calls}) — produce your final answer with the data you have"
                    )
                } else {
                    executions += 1;
                    execute_tool(tools, call)
                };
                messages.push(Message::Tool {
                    call_id: call.id.clone(),
                    content: result,
                });
            }
        }
        if exhausted {
            messages.push(Message::User(format!(
                "Tool call budget exhausted (max {max_tool_calls}) — produce your final answer with the data you have."
            )));
            let new = trace.new_messages(messages);
            trace_messages(new, *trace);
            let mut last = backend.complete(messages, tools)?;
            trace_message(&last, *trace);
            last.tool_calls = Vec::new();
            messages.push(Message::Assistant(last.clone()));
            return Ok(AgentOutcome::Exhausted(last));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{EchoTool, FailTool, ScriptedBackend};
    use crate::tools::Tool;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    fn tc(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args.to_string(),
        }
    }

    fn text(content: &str) -> AssistantMessage {
        AssistantMessage::text(content)
    }

    fn tools() -> Vec<&'static dyn Tool> {
        vec![&EchoTool, &FailTool]
    }

    #[test]
    fn test_message_api_json() {
        let msg = Message::Assistant(AssistantMessage::with_tools(
            None,
            vec![tc("call_1", "echo", "{}")],
        ));
        let v = msg.to_api_json();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["content"], serde_json::Value::Null);
        assert_eq!(v["tool_calls"][0]["id"], "call_1");
        assert_eq!(v["tool_calls"][0]["function"]["name"], "echo");

        let msg = Message::Tool {
            call_id: "call_1".to_string(),
            content: "result".to_string(),
        };
        let v = msg.to_api_json();
        assert_eq!(v["role"], "tool");
        assert_eq!(v["tool_call_id"], "call_1");
    }

    #[test]
    fn test_parse_response_content_and_reasoning() {
        let raw = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "hello",
                    "reasoning_content": "think",
                }
            }]
        });
        let msg = parse_response(&raw).unwrap();
        assert_eq!(msg.content.as_deref(), Some("hello"));
        assert_eq!(msg.reasoning.as_deref(), Some("think"));
        assert!(msg.tool_calls.is_empty());
    }

    #[test]
    fn test_parse_response_openrouter_reasoning() {
        let raw = serde_json::json!({
            "choices": [{ "message": { "content": null, "reasoning": "r" } }]
        });
        let msg = parse_response(&raw).unwrap();
        assert_eq!(msg.content, None);
        assert_eq!(msg.reasoning.as_deref(), Some("r"));
    }

    #[test]
    fn test_parse_response_tool_calls() {
        let raw = serde_json::json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "c1",
                        "type": "function",
                        "function": { "name": "echo", "arguments": "{\"queries\":[\"rice\"]}" }
                    }]
                }
            }]
        });
        let msg = parse_response(&raw).unwrap();
        assert_eq!(msg.tool_calls.len(), 1);
        assert_eq!(msg.tool_calls[0].id, "c1");
        assert_eq!(msg.tool_calls[0].name, "echo");
        assert_eq!(msg.tool_calls[0].arguments, "{\"queries\":[\"rice\"]}");
    }

    #[test]
    fn test_parse_response_missing_choices_errors() {
        let raw = serde_json::json!({ "error": "boom" });
        assert!(parse_response(&raw).is_err());
    }

    #[test]
    fn test_agent_loop_tool_roundtrip() {
        let backend = ScriptedBackend::new(vec![
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c1", "echo", "{\"q\":1}")],
            )),
            Ok(text("final answer")),
        ]);
        let mut messages = vec![Message::User("hi".to_string())];
        let outcome =
            run_agent_loop(&backend, &mut messages, &tools(), 20, &mut Trace::default()).unwrap();
        assert_eq!(outcome, AgentOutcome::Final(text("final answer")));
        assert_eq!(messages.len(), 4);
        match &messages[1] {
            Message::Assistant(m) => assert_eq!(m.tool_calls.len(), 1),
            _ => panic!("expected assistant message"),
        }
        match &messages[2] {
            Message::Tool { call_id, content } => {
                assert_eq!(call_id, "c1");
                assert_eq!(content, "{\"q\":1}");
            }
            _ => panic!("expected tool message"),
        }
    }

    #[test]
    fn test_agent_loop_multiple_tool_calls_feed_back() {
        let backend = ScriptedBackend::new(vec![
            Ok(AssistantMessage::with_tools(
                None,
                vec![
                    tc("c1", "echo", "{\"a\":1}"),
                    tc("c2", "fail_tool", "{\"a\":2}"),
                ],
            )),
            Ok(text("done")),
        ]);
        let mut messages = vec![Message::User("hi".to_string())];
        run_agent_loop(&backend, &mut messages, &tools(), 20, &mut Trace::default()).unwrap();
        let tool_msgs: Vec<&str> = messages
            .iter()
            .filter_map(|m| match m {
                Message::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tool_msgs.len(), 2);
        assert_eq!(tool_msgs[0], "{\"a\":1}");
        assert_eq!(tool_msgs[1], "error: tool exploded");
    }

    #[test]
    fn test_agent_loop_failed_calls_count_against_budget() {
        let backend = ScriptedBackend::new(vec![
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c1", "fail_tool", "{}")],
            )),
            // The round where the budget is already spent is discarded, then
            // one more response is taken unconditionally as the final answer.
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c2", "fail_tool", "{}")],
            )),
            Ok(text("final answer")),
        ]);
        let mut messages = vec![Message::User("hi".to_string())];
        let outcome =
            run_agent_loop(&backend, &mut messages, &tools(), 1, &mut Trace::default()).unwrap();
        let AgentOutcome::Exhausted(last) = outcome else {
            panic!("expected exhaustion");
        };
        assert_eq!(last.content.as_deref(), Some("final answer"));
        assert!(last.tool_calls.is_empty());
        let tool_msgs: Vec<&str> = messages
            .iter()
            .filter_map(|m| match m {
                Message::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tool_msgs.len(), 1, "no further tool executes");
        assert_eq!(tool_msgs[0], "error: tool exploded");
        let budget_messages: Vec<&String> = messages
            .iter()
            .filter_map(|m| match m {
                Message::User(s) if s.contains("budget exhausted") => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(budget_messages.len(), 1);
        assert_eq!(backend.record().len(), 3);
    }

    #[test]
    fn test_agent_loop_unknown_tool_fed_back() {
        let backend = ScriptedBackend::new(vec![
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c1", "nope", "{}")],
            )),
            Ok(text("done")),
        ]);
        let mut messages = vec![Message::User("hi".to_string())];
        run_agent_loop(&backend, &mut messages, &tools(), 20, &mut Trace::default()).unwrap();
        match &messages[2] {
            Message::Tool { content, .. } => assert_eq!(content, "error: no such tool 'nope'"),
            _ => panic!("expected tool message"),
        }
    }

    #[test]
    fn test_agent_loop_budget_exhaustion() {
        let backend = ScriptedBackend::new(vec![
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c1", "echo", "{}")],
            )),
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c2", "echo", "{}")],
            )),
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c3", "echo", "{}")],
            )),
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c4", "echo", "{}")],
            )),
            Ok(text("answer anyway")),
        ]);
        let mut messages = vec![Message::User("hi".to_string())];
        let outcome =
            run_agent_loop(&backend, &mut messages, &tools(), 3, &mut Trace::default()).unwrap();
        let AgentOutcome::Exhausted(last) = outcome else {
            panic!("expected exhaustion");
        };
        assert_eq!(last.content.as_deref(), Some("answer anyway"));
        let budget_messages: Vec<&String> = messages
            .iter()
            .filter_map(|m| match m {
                Message::User(s) if s.contains("budget exhausted") => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(budget_messages.len(), 1);
        assert!(budget_messages[0].contains("budget exhausted (max 3)"));
        let executed = messages
            .iter()
            .filter(|m| matches!(m, Message::Tool { .. }))
            .count();
        assert_eq!(executed, 3);
    }

    #[test]
    fn test_agent_loop_mid_batch_budget_exhaustion_answers_all_calls() {
        let backend = ScriptedBackend::new(vec![
            Ok(AssistantMessage::with_tools(
                None,
                vec![
                    tc("c1", "echo", "{}"),
                    tc("c2", "echo", "{}"),
                    tc("c3", "echo", "{}"),
                    tc("c4", "echo", "{}"),
                ],
            )),
            Ok(text("answer anyway")),
        ]);
        let mut messages = vec![Message::User("hi".to_string())];
        let outcome =
            run_agent_loop(&backend, &mut messages, &tools(), 3, &mut Trace::default()).unwrap();
        let AgentOutcome::Exhausted(last) = outcome else {
            panic!("expected exhaustion");
        };
        assert_eq!(last.content.as_deref(), Some("answer anyway"));
        let tool_msgs: Vec<&str> = messages
            .iter()
            .filter_map(|m| match m {
                Message::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tool_msgs.len(), 4, "every tool call must be answered");
        let executed = tool_msgs
            .iter()
            .filter(|c| !c.starts_with("skipped:"))
            .count();
        assert_eq!(executed, 3);
        assert!(tool_msgs[3].contains("skipped: tool call budget exhausted"));
        let budget_messages: Vec<&String> = messages
            .iter()
            .filter_map(|m| match m {
                Message::User(s) if s.contains("budget exhausted") => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(budget_messages.len(), 1);
    }

    #[test]
    fn test_agent_loop_exhausted_response_with_tools_taken_unconditionally() {
        let backend = ScriptedBackend::new(vec![
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c1", "echo", "{}")],
            )),
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c2", "echo", "{}")],
            )),
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c3", "echo", "{}")],
            )),
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c4", "echo", "{}")],
            )),
            Ok(AssistantMessage::with_tools(
                Some("partial"),
                vec![tc("c5", "echo", "{}")],
            )),
        ]);
        let mut messages = vec![Message::User("hi".to_string())];
        let outcome =
            run_agent_loop(&backend, &mut messages, &tools(), 3, &mut Trace::default()).unwrap();
        let AgentOutcome::Exhausted(last) = outcome else {
            panic!("expected exhaustion");
        };
        assert_eq!(last.content.as_deref(), Some("partial"));
        assert!(last.tool_calls.is_empty());
        assert_eq!(backend.record().len(), 5);
        let Message::Assistant(stored) = messages.last().unwrap() else {
            panic!("expected stored assistant message");
        };
        assert!(stored.tool_calls.is_empty());
    }

    #[test]
    fn test_agent_loop_no_tools_when_empty() {
        let backend = ScriptedBackend::new(vec![Ok(text("done"))]);
        let mut messages = vec![Message::User("hi".to_string())];
        run_agent_loop(&backend, &mut messages, &[], 20, &mut Trace::default()).unwrap();
        let (_, tool_names) = &backend.record()[0];
        assert!(tool_names.is_empty());
    }

    #[test]
    fn test_agent_loop_trace_both_runs() {
        let backend = ScriptedBackend::new(vec![
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c1", "echo", "{\"q\":1}")],
            )),
            Ok(text("final answer")),
        ]);
        let mut messages = vec![
            Message::System("system prompt".to_string()),
            Message::User("hi".to_string()),
        ];
        let mut trace = Trace::new(true, true, false);
        let outcome = run_agent_loop(&backend, &mut messages, &tools(), 20, &mut trace).unwrap();
        assert_eq!(outcome, AgentOutcome::Final(text("final answer")));
        assert_eq!(backend.record().len(), 2);
        assert_eq!(messages.len(), 5);
    }

    #[test]
    fn test_agent_loop_trace_requests_only_runs() {
        let backend = ScriptedBackend::new(vec![
            Ok(AssistantMessage::with_tools(
                None,
                vec![tc("c1", "echo", "{\"q\":1}")],
            )),
            Ok(text("final answer")),
        ]);
        let mut messages = vec![Message::User("hi".to_string())];
        let mut trace = Trace::new(true, false, false);
        run_agent_loop(&backend, &mut messages, &tools(), 20, &mut trace).unwrap();
        assert_eq!(backend.record().len(), 2);
    }

    #[test]
    fn test_trace_new_messages_only_once() {
        let mut trace = Trace::new(true, false, false);
        let mut messages = vec![Message::User("a".to_string())];
        let first = trace.new_messages(&messages);
        assert_eq!(first, &messages[..]);
        assert!(trace.new_messages(&messages).is_empty());
        messages.push(Message::User("b".to_string()));
        let second = trace.new_messages(&messages);
        assert_eq!(second, &[Message::User("b".to_string())]);
        messages.push(Message::System("s".to_string()));
        messages.push(Message::User("c".to_string()));
        let third = trace.new_messages(&messages);
        assert_eq!(
            third,
            &[
                Message::System("s".to_string()),
                Message::User("c".to_string())
            ]
        );
    }

    fn capture_messages(messages: &[Message], trace: Trace) -> String {
        let mut out = Vec::new();
        write_trace_messages(&mut out, messages, trace);
        String::from_utf8(out).unwrap()
    }

    fn capture_response(msg: &AssistantMessage, trace: Trace) -> String {
        let mut out = Vec::new();
        write_trace_message(&mut out, msg, trace);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn test_trace_requests_role_prefixed_lines() {
        let messages = [
            Message::System("sys".to_string()),
            Message::User("hi".to_string()),
            Message::Assistant(AssistantMessage::with_tools(
                None,
                vec![tc("c1", "usda_search", "{\"queries\":[\"rice\"]}")],
            )),
            Message::Tool {
                call_id: "c1".to_string(),
                content: "result".to_string(),
            },
        ];
        let out = capture_messages(&messages, Trace::new(true, false, false));
        assert!(out.contains("--- to model ---"), "got: {out}");
        assert!(out.contains("[system] sys"), "got: {out}");
        assert!(out.contains("[user] hi"), "got: {out}");
        assert!(
            out.contains("[assistant] (tool calls: usda_search)"),
            "got: {out}"
        );
        assert!(out.contains("[tool:c1] result"), "got: {out}");
        assert!(out.contains("--- end to model ---"), "got: {out}");
        assert!(!out.contains("from model"), "responses toggle off: {out}");
        assert!(!out.contains("\x1b["), "no ANSI with colors off: {out}");
    }

    #[test]
    fn test_trace_requests_prints_only_new_messages() {
        let mut trace = Trace::new(true, false, false);
        let mut messages = vec![Message::User("a".to_string())];
        let first = capture_messages(&trace.new_messages(&messages), trace);
        assert!(first.contains("[user] a"), "got: {first}");
        messages.push(Message::User("b".to_string()));
        let second = capture_messages(&trace.new_messages(&messages), trace);
        assert!(
            !second.contains("[user] a"),
            "old message reprinted: {second}"
        );
        assert!(second.contains("[user] b"), "got: {second}");
    }

    #[test]
    fn test_trace_responses_reasoning_tools_and_output() {
        let msg = AssistantMessage {
            content: Some("final toml".to_string()),
            tool_calls: vec![tc("c1", "usda_search", "{\"queries\":[\"rice\"]}")],
            reasoning: Some("think step".to_string()),
        };
        let out = capture_response(&msg, Trace::new(false, true, false));
        assert!(out.contains("--- from model ---"), "got: {out}");
        assert!(out.contains("[reasoning] think step"), "got: {out}");
        assert!(
            out.contains("[tool] usda_search {\"queries\":[\"rice\"]}"),
            "got: {out}"
        );
        assert!(out.contains("final toml"), "got: {out}");
        assert!(out.contains("--- end from model ---"), "got: {out}");
        assert!(!out.contains("to model"), "requests toggle off: {out}");
        assert!(!out.contains("\x1b["), "no ANSI with colors off: {out}");
    }

    #[test]
    fn test_trace_toggles_independent() {
        let msg = AssistantMessage::text("x");
        assert!(
            capture_response(&msg, Trace::new(false, false, false)).is_empty(),
            "all toggles off prints nothing"
        );
        assert!(!capture_response(&msg, Trace::new(false, true, false)).is_empty());
        let messages = [Message::User("x".to_string())];
        assert!(
            capture_messages(&messages, Trace::new(false, true, false)).is_empty(),
            "responses-only must not print requests"
        );
        assert!(!capture_messages(&messages, Trace::new(true, false, false)).is_empty());
    }

    #[test]
    fn test_trace_colors_markers_and_lines() {
        let msg = AssistantMessage::text("x");
        let out = capture_response(&msg, Trace::new(false, true, true));
        assert!(out.contains("\x1b[1;33m"), "bold yellow marker: {out}");
        assert!(out.contains("\x1b[32m"), "green response line: {out}");
        assert!(out.contains("\x1b[0m"), "reset: {out}");
        let messages = [Message::User("x".to_string())];
        let out = capture_messages(&messages, Trace::new(true, false, true));
        assert!(out.contains("\x1b[36m"), "cyan request line: {out}");
    }

    #[test]
    fn test_function_definitions_shape() {
        let defs = function_definitions(&[&EchoTool]);
        assert_eq!(defs[0]["type"], "function");
        assert_eq!(defs[0]["function"]["name"], "echo");
        assert_eq!(defs[0]["function"]["parameters"]["type"], "object");
    }

    fn serve_once(status: u16, body: String, tx: mpsc::Sender<()>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = if body.is_empty() {
                "HTTP/1.1 429 Too Many Requests\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                    .to_string()
            } else {
                let payload = format!("{{\"choices\":[{{\"message\":{body}}}]}}");
                format!(
                    "HTTP/1.1 {status} OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    payload.len(),
                    payload
                )
            };
            let _ = stream.write_all(response.as_bytes());
            let _ = tx.send(());
        });
        format!("http://{addr}/v1")
    }

    #[test]
    fn test_real_backend_parses_response() {
        let (tx, rx) = mpsc::channel();
        let base = serve_once(
            200,
            "{\"content\":\"works\",\"reasoning_content\":\"r\"}".to_string(),
            tx,
        );
        let mut settings = AiSettings::default();
        settings.base_url = base;
        let backend = OpenAiCompatible::with_tuning(&settings, Duration::from_millis(1), None);
        let msg = backend
            .complete(&[Message::User("hi".to_string())], &[])
            .unwrap();
        assert_eq!(msg.content.as_deref(), Some("works"));
        assert_eq!(msg.reasoning.as_deref(), Some("r"));
        rx.recv().unwrap();
    }

    #[test]
    fn test_real_backend_retries_429_then_succeeds() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for (i, result) in listener.incoming().take(2).enumerate() {
                let mut stream = result.unwrap();
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let response = if i == 0 {
                    "HTTP/1.1 429 Too Many Requests\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".to_string()
                } else {
                    let payload = "{\"choices\":[{\"message\":{\"content\":\"ok\"}}]}";
                    format!(
                        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        payload.len(),
                        payload
                    )
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let mut settings = AiSettings::default();
        settings.base_url = format!("http://{addr}/v1");
        let backend = OpenAiCompatible::with_tuning(&settings, Duration::from_millis(1), None);
        let msg = backend
            .complete(&[Message::User("hi".to_string())], &[])
            .unwrap();
        assert_eq!(msg.content.as_deref(), Some("ok"));
    }

    #[test]
    fn test_real_backend_gives_up_after_retries() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for result in listener.incoming().take(3) {
                let mut stream = result.unwrap();
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 500 Boom\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                );
            }
        });
        let mut settings = AiSettings::default();
        settings.base_url = format!("http://{addr}/v1");
        let backend = OpenAiCompatible::with_tuning(&settings, Duration::from_millis(1), None);
        let err = backend
            .complete(&[Message::User("hi".to_string())], &[])
            .unwrap_err();
        assert!(matches!(err, LlmError::Http { status: 500, .. }));
    }

    #[test]
    fn test_real_backend_timeout_aborts() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            thread::sleep(Duration::from_secs(5));
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        });
        let mut settings = AiSettings::default();
        settings.base_url = format!("http://{addr}/v1");
        let backend = OpenAiCompatible::with_tuning(
            &settings,
            Duration::from_millis(1),
            Some(Duration::from_millis(100)),
        );
        let err = backend
            .complete(&[Message::User("hi".to_string())], &[])
            .unwrap_err();
        assert!(matches!(err, LlmError::Timeout));
    }

    #[test]
    fn test_llm_error_message_tools_hint() {
        let err = LlmError::Http {
            status: 400,
            body: "Unknown parameter: 'tools'".to_string(),
        };
        assert!(err.message().contains("tool calling"));
        let err = LlmError::Http {
            status: 400,
            body: "bad request".to_string(),
        };
        assert!(!err.message().contains("tool calling"));
    }
}
