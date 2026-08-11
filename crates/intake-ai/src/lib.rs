//! A generic, blocking, OpenAI-compatible agent pipeline with a
//! human-in-the-loop resolve stage.
//!
//! `intake-ai` takes text in and returns a validated, confirmed value: run
//! a conversation with a model, let it use caller-registered tools, parse
//! its final answer into `T`, and only hand it back once a confirmer has
//! accepted it. It knows nothing about intake, food, nutrition, or TOML —
//! the consumer supplies the parse closures, prompt templates, tools, and
//! confirmation UX.
//!
//! It is deliberately blocking and minimal: no async runtime, no client
//! abstractions beyond an OpenAI-compatible `POST /chat/completions`, and
//! nothing but `serde_json` and `ureq` on the dependency tree. (`serde`
//! itself comes in transitively via `serde_json`.) Construct a
//! [`settings::Settings`] with its `new` constructor — `model` and
//! `base_url` are required, `api_key` is optional (omit it for endpoints
//! without auth) — and point `base_url` at any OpenAI-compatible
//! endpoint (OpenAI, Groq, Mistral, OpenRouter, Ollama, vLLM, …).
//!
//! # Pipeline
//!
//! - [`llm::LlmBackend`] — the model interface; [`llm::OpenAiCompatible`]
//!   is the real client, tests use a scripted fake.
//! - [`pipeline::Resolver`] — the resolve loop: agent loop → parse →
//!   confirm, retrying on parse failures and looping on user feedback.
//! - [`confirm::Confirmer`] — the only terminal hook; implementations live
//!   at the consumer.
//!
//! # Tracing
//!
//! The library emits structured [`llm::TraceEvent`]s (messages sent,
//! responses, parse errors) through a [`llm::TraceObserver`]; rendering is
//! the consumer's business. Here is a minimal observer that prints requests
//! to stderr:
//!
//! ```
//! use intake_ai::llm::{Message, TraceEvent, TraceObserver};
//!
//! struct StderrObserver;
//!
//! impl TraceObserver for StderrObserver {
//!     fn on_event(&self, event: &TraceEvent<'_>) {
//!         match event {
//!             TraceEvent::MessagesSent(messages) => {
//!                 eprintln!("--- to model ---");
//!                 for message in *messages {
//!                     let line = match message {
//!                         Message::System(c) => format!("[system] {c}"),
//!                         Message::User(c) => format!("[user] {c}"),
//!                         _ => String::new(),
//!                     };
//!                     if !line.is_empty() {
//!                         eprintln!("{line}");
//!                     }
//!                 }
//!                 eprintln!("--- end to model ---");
//!             }
//!             _ => {}
//!         }
//!     }
//! }
//! ```

pub mod confirm;
pub mod llm;
pub mod pipeline;
pub mod settings;
pub mod tools;

#[cfg(test)]
pub(crate) mod testing;
