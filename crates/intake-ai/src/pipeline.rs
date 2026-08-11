use crate::confirm::{ConfirmDecision, Confirmer};
use crate::llm::{
    run_agent_loop, AgentOutcome, AssistantMessage, LlmBackend, LlmError, Message, TraceEvent,
    TraceObserver,
};
use crate::settings::Settings;
use crate::tools::Tool;
use std::sync::Arc;

#[derive(Debug)]
pub enum ResolveError {
    Exhausted {
        last_error: String,
        raw_output: String,
    },
    Rejected,
    /// An error from the pipeline's internals — the LLM backend or the
    /// confirmer — boxed and passed through unchanged.
    Internal(Box<dyn std::error::Error + Send + Sync>),
}

impl From<LlmError> for ResolveError {
    fn from(e: LlmError) -> Self {
        ResolveError::Internal(Box::new(e))
    }
}

pub struct ResolveContext<'a> {
    pub settings: &'a Settings,
    pub backend: &'a dyn LlmBackend,
    pub tools: &'a [&'a dyn Tool],
    /// Where trace events go; when None, events are dropped. Callers that
    /// share stderr with other output (such as a progress line) can inject
    /// an observer that serializes rendering with it.
    pub trace_observer: Option<Arc<dyn TraceObserver>>,
    /// When true, the resolved value is returned without rendering the
    /// proposal or consulting the confirmer.
    pub auto_accept: bool,
}

pub struct Resolver<'ctx, 'a> {
    ctx: &'ctx ResolveContext<'a>,
    confirm: &'ctx mut dyn Confirmer,
    system: String,
}

fn fence_strip(s: &str) -> String {
    let s = s.trim();
    if !s.starts_with("```") {
        return s.to_string();
    }
    let mut lines = s.lines();
    lines.next();
    let mut out: Vec<&str> = Vec::new();
    for line in lines {
        if line.trim() == "```" {
            break;
        }
        out.push(line);
    }
    out.join("\n")
}

impl<'ctx, 'a> Resolver<'ctx, 'a> {
    pub fn new(
        ctx: &'ctx ResolveContext<'a>,
        confirm: &'ctx mut dyn Confirmer,
        system: String,
    ) -> Resolver<'ctx, 'a> {
        Resolver {
            ctx,
            confirm,
            system,
        }
    }

    /// Resolves `user` into a `T`: runs the agent loop, parses the final
    /// answer with `parse`, and confirms with the confirmer, retrying on
    /// parse failures up to `max_retries`. Trace events go to the
    /// context's observer; observers are infallible, so tracing never
    /// affects the result.
    pub fn resolve<T>(
        &mut self,
        user: &str,
        parse: impl Fn(&str) -> Result<T, String>,
        present: &dyn Fn(&T) -> String,
    ) -> Result<T, ResolveError> {
        let mut messages = vec![
            Message::System(self.system.clone()),
            Message::User(user.to_string()),
        ];
        let mut retries_left = self.ctx.settings.max_retries;
        let mut no_tools = false;
        let mut trace = crate::llm::Trace::new(
            self.ctx.settings.trace_requests,
            self.ctx.settings.trace_responses,
        );
        if let Some(observer) = &self.ctx.trace_observer {
            trace = trace.with_observer(Arc::clone(observer));
        }

        loop {
            let tools: &[&dyn Tool] = if no_tools { &[] } else { self.ctx.tools };
            let outcome = run_agent_loop(
                self.ctx.backend,
                &mut messages,
                tools,
                self.ctx.settings.max_tool_calls,
                &mut trace,
            )?;
            if matches!(outcome, AgentOutcome::Exhausted(_)) {
                no_tools = true;
            }
            let final_msg: &AssistantMessage = match &outcome {
                AgentOutcome::Final(m) => m,
                AgentOutcome::Exhausted(m) => m,
            };
            let content = final_msg.content.clone().unwrap_or_default();
            let stripped = fence_strip(&content);

            let value = match parse(&stripped) {
                Ok(value) => value,
                Err(e) => {
                    if retries_left == 0 {
                        return Err(ResolveError::Exhausted {
                            last_error: e,
                            raw_output: stripped,
                        });
                    }
                    retries_left -= 1;
                    trace.emit(&TraceEvent::ParseError(&e));
                    let mut note = String::new();
                    if no_tools {
                        note = "\nTool calls are no longer available — produce your final answer from the data already gathered."
                            .to_string();
                    }
                    messages.push(Message::User(format!(
                        "Your previous output failed to parse or validate: {e}\n\
                         Fix the error and reply with only the corrected output.{note}"
                    )));
                    continue;
                }
            };

            if self.ctx.auto_accept {
                return Ok(value);
            }
            let rendered = present(&value);
            match self.confirm.confirm(&rendered) {
                Ok(ConfirmDecision::Accept) => return Ok(value),
                Ok(ConfirmDecision::Reject) => return Err(ResolveError::Rejected),
                Ok(ConfirmDecision::Feedback(msg)) => {
                    messages.push(Message::User(format!("User feedback: {msg}")));
                    retries_left = self.ctx.settings.max_retries;
                    no_tools = false;
                }
                Err(e) => return Err(ResolveError::Internal(e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirm::ConfirmDecision;
    use crate::testing::{ScriptedBackend, ScriptedConfirmer};

    fn settings() -> Settings {
        Settings::new("http://test/v1", "m", None)
    }

    fn with_resolver<F, T>(
        settings: &Settings,
        backend: &dyn LlmBackend,
        confirmer: &mut dyn Confirmer,
        tools: &[&dyn Tool],
        f: F,
    ) -> T
    where
        F: FnOnce(&mut Resolver<'_, '_>) -> T,
    {
        let ctx = ResolveContext {
            settings,
            backend,
            tools,
            trace_observer: None,
            auto_accept: false,
        };
        let mut resolver = Resolver::new(&ctx, confirmer, "system".to_string());
        f(&mut resolver)
    }

    fn resolve_int(
        settings: &Settings,
        backend: &dyn LlmBackend,
        confirmer: &mut dyn Confirmer,
        tools: &[&dyn Tool],
        user: &str,
    ) -> Result<i64, ResolveError> {
        with_resolver(settings, backend, confirmer, tools, |r| {
            r.resolve(user, parse_int, &present)
        })
    }

    fn parse_int(s: &str) -> Result<i64, String> {
        s.trim().parse::<i64>().map_err(|e| e.to_string())
    }

    fn present(v: &i64) -> String {
        format!("proposal: {v}")
    }

    #[test]
    fn test_fence_strip() {
        assert_eq!(fence_strip("42"), "42");
        assert_eq!(fence_strip("```toml\n42\n```"), "42");
        assert_eq!(fence_strip("```\n42\n```"), "42");
        assert_eq!(fence_strip("```toml\n```"), "");
        assert_eq!(fence_strip("  ```\n  42\n  ```  "), "  42");
        assert_eq!(fence_strip("```\n42"), "42");
    }

    #[test]
    fn test_fence_strip_keeps_inner_content() {
        assert_eq!(fence_strip("```\nline1\nline2\n```"), "line1\nline2");
    }

    #[test]
    fn test_resolve_accepts_first_parse() {
        let backend = ScriptedBackend::new(vec![Ok(crate::llm::AssistantMessage::text("42"))]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Accept]);
        let s = settings();
        let value = resolve_int(&s, &backend, &mut confirmer, &[], "what is 6*7?").unwrap();
        assert_eq!(value, 42);
        assert_eq!(confirmer.rendered(), vec!["proposal: 42".to_string()]);
        assert_eq!(backend.record().len(), 1);
    }

    #[test]
    fn test_resolve_retries_bad_then_good() {
        let backend = ScriptedBackend::new(vec![
            Ok(crate::llm::AssistantMessage::text("not a number")),
            Ok(crate::llm::AssistantMessage::text("7")),
        ]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Accept]);
        let s = settings();
        let value = resolve_int(&s, &backend, &mut confirmer, &[], "x").unwrap();
        assert_eq!(value, 7);
        let (messages, _) = &backend.record()[1];
        let user_msgs: Vec<&str> = messages
            .iter()
            .filter_map(|m| match m {
                Message::User(u) => Some(u.as_str()),
                _ => None,
            })
            .collect();
        assert!(user_msgs[1].contains("failed to parse or validate"));
        assert!(user_msgs[1].contains("invalid digit"));
    }

    #[test]
    fn test_resolve_exhaustion_returns_error_and_raw_output() {
        let backend = ScriptedBackend::new(vec![
            Ok(crate::llm::AssistantMessage::text("bad1")),
            Ok(crate::llm::AssistantMessage::text("bad2")),
            Ok(crate::llm::AssistantMessage::text("bad3")),
            Ok(crate::llm::AssistantMessage::text("bad4")),
        ]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Accept]);
        let mut s = settings();
        s.max_retries = 3;
        let err = resolve_int(&s, &backend, &mut confirmer, &[], "x").unwrap_err();
        match err {
            ResolveError::Exhausted {
                last_error,
                raw_output,
            } => {
                assert_eq!(raw_output, "bad4");
                assert!(last_error.contains("invalid digit"));
            }
            _ => panic!("expected exhaustion"),
        }
        assert_eq!(backend.record().len(), 4);
    }

    #[test]
    fn test_resolve_reject_maps_to_error() {
        let backend = ScriptedBackend::new(vec![Ok(crate::llm::AssistantMessage::text("1"))]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Reject]);
        let s = settings();
        let err = resolve_int(&s, &backend, &mut confirmer, &[], "x").unwrap_err();
        assert!(matches!(err, ResolveError::Rejected));
    }

    #[test]
    fn test_resolve_feedback_continues_conversation() {
        let backend = ScriptedBackend::new(vec![
            Ok(crate::llm::AssistantMessage::text("1")),
            Ok(crate::llm::AssistantMessage::text("2")),
        ]);
        let mut confirmer = ScriptedConfirmer::new(vec![
            ConfirmDecision::Feedback("make it bigger".to_string()),
            ConfirmDecision::Accept,
        ]);
        let s = settings();
        let value = resolve_int(&s, &backend, &mut confirmer, &[], "x").unwrap();
        assert_eq!(value, 2);
        assert_eq!(confirmer.rendered().len(), 2);
        let (messages, _) = &backend.record()[1];
        let user_msgs: Vec<&str> = messages
            .iter()
            .filter_map(|m| match m {
                Message::User(u) => Some(u.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(user_msgs.last().unwrap(), &"User feedback: make it bigger");
    }

    #[test]
    fn test_resolve_feedback_restarts_budget_and_retries() {
        let backend = ScriptedBackend::new(vec![
            Ok(crate::llm::AssistantMessage::text("bad")),
            Ok(crate::llm::AssistantMessage::text("1")),
            Ok(crate::llm::AssistantMessage::text("bad")),
            Ok(crate::llm::AssistantMessage::text("2")),
        ]);
        let mut confirmer = ScriptedConfirmer::new(vec![
            ConfirmDecision::Feedback("again".to_string()),
            ConfirmDecision::Accept,
        ]);
        let mut s = settings();
        s.max_retries = 1;
        let value = resolve_int(&s, &backend, &mut confirmer, &[], "x").unwrap();
        assert_eq!(value, 2);
        assert_eq!(backend.record().len(), 4);
    }

    #[test]
    fn test_resolve_confirmer_error_maps_to_internal() {
        let backend = ScriptedBackend::new(vec![Ok(crate::llm::AssistantMessage::text("1"))]);
        let mut confirmer = ScriptedConfirmer::results(vec![Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "confirmation broke",
        )))]);
        let s = settings();
        let err = resolve_int(&s, &backend, &mut confirmer, &[], "x").unwrap_err();
        let ResolveError::Internal(e) = err else {
            panic!("expected Internal error");
        };
        assert!(e.to_string().contains("confirmation broke"));
    }

    #[test]
    fn test_resolve_auto_accept_skips_confirmation() {
        let backend = ScriptedBackend::new(vec![Ok(crate::llm::AssistantMessage::text("1"))]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Reject]);
        let s = settings();
        let ctx = ResolveContext {
            settings: &s,
            backend: &backend,
            tools: &[],
            trace_observer: None,
            auto_accept: true,
        };
        let mut resolver = Resolver::new(&ctx, &mut confirmer, "system".to_string());
        let value = resolver.resolve("x", parse_int, &present).unwrap();
        assert_eq!(value, 1);
        assert!(confirmer.rendered().is_empty());
    }

    #[test]
    fn test_resolve_parse_retries_carry_no_tools_after_exhaustion() {
        let backend = ScriptedBackend::new(vec![
            Ok(crate::llm::AssistantMessage::with_tools(
                None,
                vec![crate::llm::ToolCall {
                    id: "c1".to_string(),
                    name: "echo".to_string(),
                    arguments: "{}".to_string(),
                }],
            )),
            Ok(crate::llm::AssistantMessage::with_tools(
                None,
                vec![crate::llm::ToolCall {
                    id: "c2".to_string(),
                    name: "echo".to_string(),
                    arguments: "{}".to_string(),
                }],
            )),
            Ok(crate::llm::AssistantMessage::with_tools(
                None,
                vec![crate::llm::ToolCall {
                    id: "c3".to_string(),
                    name: "echo".to_string(),
                    arguments: "{}".to_string(),
                }],
            )),
            Ok(crate::llm::AssistantMessage::text("not a number")),
            Ok(crate::llm::AssistantMessage::text("9")),
        ]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Accept]);
        let mut s = settings();
        s.max_tool_calls = 2;
        s.max_retries = 1;
        let echo = crate::testing::EchoTool;
        let tools: &[&dyn Tool] = &[&echo];
        let value = resolve_int(&s, &backend, &mut confirmer, tools, "x").unwrap();
        assert_eq!(value, 9);
        let record = backend.record();
        assert_eq!(record.len(), 5);
        let (last_messages, last_tools) = &record[record.len() - 1];
        assert!(last_tools.is_empty());
        let user_msgs: Vec<&str> = last_messages
            .iter()
            .filter_map(|m| match m {
                Message::User(u) => Some(u.as_str()),
                _ => None,
            })
            .collect();
        assert!(user_msgs
            .iter()
            .any(|u| u.contains("Tool call budget exhausted")));
        assert!(user_msgs
            .iter()
            .any(|u| u.contains("Tool calls are no longer available")));
    }

    #[test]
    fn test_resolve_trace_responses_writes_stderr() {
        let backend = ScriptedBackend::new(vec![Ok(crate::llm::AssistantMessage {
            content: Some("7".to_string()),
            tool_calls: vec![],
            reasoning: Some("think step".to_string()),
        })]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Accept]);
        let mut s = settings();
        s.trace_responses = true;
        let value = resolve_int(&s, &backend, &mut confirmer, &[], "x").unwrap();
        assert_eq!(value, 7);
    }

    #[test]
    fn test_resolve_trace_requests_writes_stderr() {
        let backend = ScriptedBackend::new(vec![Ok(crate::llm::AssistantMessage::text("7"))]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Accept]);
        let mut s = settings();
        s.trace_requests = true;
        let value = resolve_int(&s, &backend, &mut confirmer, &[], "x").unwrap();
        assert_eq!(value, 7);
    }

    #[test]
    fn test_resolve_trace_requests_emits_messages_sent() {
        let backend = ScriptedBackend::new(vec![Ok(crate::llm::AssistantMessage::text("7"))]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Accept]);
        let mut s = settings();
        s.trace_requests = true;
        let observer = Arc::new(crate::testing::RecordingObserver::new());
        let ctx = ResolveContext {
            settings: &s,
            backend: &backend,
            tools: &[],
            trace_observer: Some(Arc::clone(&observer) as Arc<dyn crate::llm::TraceObserver>),
            auto_accept: false,
        };
        let mut resolver = Resolver::new(&ctx, &mut confirmer, "system".to_string());
        let value = resolver
            .resolve(
                "x",
                |s: &str| s.trim().parse::<i64>().map_err(|e| e.to_string()),
                &|v: &i64| format!("proposal: {v}"),
            )
            .unwrap();
        assert_eq!(value, 7);
        let events = observer.events();
        let crate::testing::OwnedTraceEvent::MessagesSent(first) = &events[0] else {
            panic!("first event must be MessagesSent: {events:?}");
        };
        assert!(
            first
                .iter()
                .any(|m| matches!(m, Message::System(s) if s == "system")),
            "request trace must carry the system message: {events:?}"
        );
        assert!(
            events
                .iter()
                .all(|e| matches!(e, crate::testing::OwnedTraceEvent::MessagesSent(_))),
            "responses toggle off, only MessagesSent expected: {events:?}"
        );
    }

    #[test]
    fn test_resolve_trace_parse_error_emitted() {
        let backend = ScriptedBackend::new(vec![
            Ok(crate::llm::AssistantMessage::text("not a number")),
            Ok(crate::llm::AssistantMessage::text("7")),
        ]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Accept]);
        let mut s = settings();
        s.trace_responses = true;
        let observer = Arc::new(crate::testing::RecordingObserver::new());
        let ctx = ResolveContext {
            settings: &s,
            backend: &backend,
            tools: &[],
            trace_observer: Some(Arc::clone(&observer) as Arc<dyn crate::llm::TraceObserver>),
            auto_accept: false,
        };
        let mut resolver = Resolver::new(&ctx, &mut confirmer, "system".to_string());
        let value = resolver.resolve("x", parse_int, &present).unwrap();
        assert_eq!(value, 7);
        let events = observer.events();
        assert!(
            events.iter().any(
                |e| matches!(e, crate::testing::OwnedTraceEvent::ParseError(e)
                    if e.contains("invalid digit"))
            ),
            "parse error must be emitted as an event: {events:?}"
        );
    }

    #[test]
    fn test_resolve_backend_error_maps_to_io() {
        let backend = ScriptedBackend::new(vec![Err(LlmError::Timeout)]);
        let mut confirmer = ScriptedConfirmer::new(vec![ConfirmDecision::Accept]);
        let s = settings();
        let err = resolve_int(&s, &backend, &mut confirmer, &[], "x").unwrap_err();
        let ResolveError::Internal(e) = err else {
            panic!("expected Internal error");
        };
        assert!(e.to_string().contains("timed out"));
    }
}
