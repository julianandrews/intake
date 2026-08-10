use crate::confirm::{ConfirmDecision, ConfirmError, Confirmer};
use crate::llm::{AssistantMessage, LlmBackend, LlmError, Message};
use crate::tools::Tool;
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

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
    queue: VecDeque<Result<ConfirmDecision, ConfirmError>>,
    rendered: Vec<String>,
    present: bool,
}

impl ScriptedConfirmer {
    pub fn new(decisions: Vec<ConfirmDecision>) -> ScriptedConfirmer {
        ScriptedConfirmer::results(decisions.into_iter().map(Ok).collect())
    }

    pub fn results(results: Vec<Result<ConfirmDecision, ConfirmError>>) -> ScriptedConfirmer {
        ScriptedConfirmer {
            queue: results.into(),
            rendered: Vec::new(),
            present: true,
        }
    }

    pub fn without_present(decisions: Vec<ConfirmDecision>) -> ScriptedConfirmer {
        ScriptedConfirmer::new(decisions).skip_present()
    }

    fn skip_present(mut self) -> ScriptedConfirmer {
        self.present = false;
        self
    }

    pub fn rendered(&self) -> Vec<String> {
        self.rendered.clone()
    }
}

impl Confirmer for ScriptedConfirmer {
    fn confirm(&mut self, rendered: &str) -> Result<ConfirmDecision, ConfirmError> {
        self.rendered.push(rendered.to_string());
        self.queue
            .pop_front()
            .unwrap_or(Ok(ConfirmDecision::Reject))
    }

    fn present_before_confirm(&self) -> bool {
        self.present
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
