use crate::confirm::{ConfirmDecision, Confirmer};
use crate::llm::{AssistantMessage, LlmBackend, LlmError, Message, TraceEvent, TraceObserver};
use crate::tools::Tool;
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

/// An owned form of [`TraceEvent`], for observers that retain events past
/// the `on_event` call. [`RecordingObserver`] copies borrowed events into
/// this on receipt, so event-level assertions don't depend on rendering.
#[derive(Debug, Clone, PartialEq)]
pub enum OwnedTraceEvent {
    MessagesSent(Vec<Message>),
    Response(AssistantMessage),
    ParseError(String),
}

/// Captures every [`TraceEvent`] a [`Trace`] emits, for event-level
/// assertions that don't depend on rendering.
///
/// [`Trace`]: crate::llm::Trace
pub struct RecordingObserver {
    events: Mutex<Vec<OwnedTraceEvent>>,
}

impl RecordingObserver {
    pub fn new() -> RecordingObserver {
        RecordingObserver {
            events: Mutex::new(Vec::new()),
        }
    }

    pub fn events(&self) -> Vec<OwnedTraceEvent> {
        self.events.lock().expect("recording observer lock").clone()
    }
}

impl Default for RecordingObserver {
    fn default() -> Self {
        RecordingObserver::new()
    }
}

impl TraceObserver for RecordingObserver {
    fn on_event(&self, event: &TraceEvent<'_>) {
        let owned = match event {
            TraceEvent::MessagesSent(messages) => OwnedTraceEvent::MessagesSent(messages.to_vec()),
            TraceEvent::Response(msg) => OwnedTraceEvent::Response((*msg).clone()),
            TraceEvent::ParseError(error) => OwnedTraceEvent::ParseError(error.to_string()),
        };
        self.events
            .lock()
            .expect("recording observer lock")
            .push(owned);
    }
}

pub struct ScriptedBackend {
    queue: Mutex<VecDeque<Result<AssistantMessage, LlmError>>>,
    calls: Mutex<Vec<(Vec<Message>, Vec<String>)>>,
}

impl ScriptedBackend {
    pub fn new(responses: Vec<Result<AssistantMessage, LlmError>>) -> ScriptedBackend {
        ScriptedBackend {
            queue: Mutex::new(responses.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn record(&self) -> Vec<(Vec<Message>, Vec<String>)> {
        self.calls().clone()
    }

    fn calls(&self) -> MutexGuard<'_, Vec<(Vec<Message>, Vec<String>)>> {
        self.calls.lock().expect("scripted backend lock")
    }
}

impl LlmBackend for ScriptedBackend {
    fn complete(
        &self,
        messages: &[Message],
        tools: &[&dyn Tool],
    ) -> Result<AssistantMessage, LlmError> {
        self.calls().push((
            messages.to_vec(),
            tools.iter().map(|t| t.name().to_string()).collect(),
        ));
        self.queue
            .lock()
            .expect("scripted backend queue lock")
            .pop_front()
            .unwrap_or_else(|| {
                Err(LlmError::Transport(
                    "scripted backend: no more responses".to_string(),
                ))
            })
    }
}

pub struct ScriptedConfirmer {
    queue: VecDeque<Result<ConfirmDecision, Box<dyn std::error::Error + Send + Sync>>>,
    rendered: Vec<String>,
}

impl ScriptedConfirmer {
    pub fn new(decisions: Vec<ConfirmDecision>) -> ScriptedConfirmer {
        ScriptedConfirmer::results(decisions.into_iter().map(Ok).collect())
    }

    pub fn results(
        results: Vec<Result<ConfirmDecision, Box<dyn std::error::Error + Send + Sync>>>,
    ) -> ScriptedConfirmer {
        ScriptedConfirmer {
            queue: results.into(),
            rendered: Vec::new(),
        }
    }

    pub fn rendered(&self) -> Vec<String> {
        self.rendered.clone()
    }
}

impl Confirmer for ScriptedConfirmer {
    fn confirm(
        &mut self,
        rendered: &str,
    ) -> Result<ConfirmDecision, Box<dyn std::error::Error + Send + Sync>> {
        self.rendered.push(rendered.to_string());
        self.queue
            .pop_front()
            .unwrap_or(Ok(ConfirmDecision::Reject))
    }
}

pub struct EchoTool;

impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "echoes the parameters back"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    fn execute(&self, params: &serde_json::Value) -> Result<String, String> {
        Ok(params.to_string())
    }
}

pub struct FailTool;

impl Tool for FailTool {
    fn name(&self) -> &str {
        "fail_tool"
    }

    fn description(&self) -> &str {
        "always fails"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    fn execute(&self, _params: &serde_json::Value) -> Result<String, String> {
        Err("tool exploded".to_string())
    }
}
