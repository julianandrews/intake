//! Terminal rendering of the [`TraceEvent`]s emitted by `intake-ai`.
//!
//! The library only decides *what* happened; presentation lives here: the
//! role-prefixed request blocks, the response blocks with reasoning and tool
//! calls, and the red parse-error lines. Writes go through the session's
//! shared writer while it is held, so each event renders atomically with
//! respect to status-line frames.

use intake_ai::llm::{AssistantMessage, Message, TraceEvent, TraceObserver};
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";

fn paint(enabled: bool, text: &str, ansi: &str) -> String {
    if enabled {
        format!("{ansi}{text}{ANSI_RESET}")
    } else {
        text.to_string()
    }
}

fn marker(enabled: bool, title: &str) -> String {
    paint(enabled, title, ANSI_BOLD_YELLOW)
}

/// Formats intake-ai's trace events into the stderr blocks described in the
/// AI design doc: role-prefixed request lines bracketed by `--- to model
/// ---`, response blocks with reasoning, tool calls, and raw output, and red
/// parse-error lines. Writes go through the session's shared writer so dumps
/// serialize with status-line frames.
pub(crate) struct TraceRenderer {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
    colors: bool,
}

impl TraceRenderer {
    pub(crate) fn new(out: Arc<Mutex<Box<dyn Write + Send>>>, colors: bool) -> TraceRenderer {
        TraceRenderer { out, colors }
    }

    fn write_messages(&self, out: &mut dyn Write, messages: &[Message]) -> io::Result<()> {
        writeln!(out, "{}", marker(self.colors, "--- to model ---"))?;
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
            writeln!(out, "{}", paint(self.colors, &line, ANSI_CYAN))?;
        }
        writeln!(out, "{}", marker(self.colors, "--- end to model ---"))?;
        writeln!(out)
    }

    fn write_response(&self, out: &mut dyn Write, msg: &AssistantMessage) -> io::Result<()> {
        writeln!(out, "{}", marker(self.colors, "--- from model ---"))?;
        if let Some(reasoning) = &msg.reasoning {
            writeln!(
                out,
                "{}",
                paint(self.colors, &format!("[reasoning] {reasoning}"), ANSI_GREEN)
            )?;
        }
        for call in &msg.tool_calls {
            writeln!(
                out,
                "{}",
                paint(
                    self.colors,
                    &format!("[tool] {} {}", call.name, call.arguments),
                    ANSI_GREEN
                )
            )?;
        }
        if let Some(content) = &msg.content {
            writeln!(out, "{}", paint(self.colors, content, ANSI_GREEN))?;
        }
        writeln!(out, "{}", marker(self.colors, "--- end from model ---"))?;
        writeln!(out)
    }

    fn write_parse_error(&self, out: &mut dyn Write, error: &str) -> io::Result<()> {
        writeln!(
            out,
            "{}",
            paint(self.colors, &format!("[parse error] {error}"), ANSI_RED)
        )
    }
}

impl TraceObserver for TraceRenderer {
    fn on_event(&self, event: &TraceEvent) {
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = match event {
            TraceEvent::MessagesSent(messages) => self.write_messages(&mut *out, messages),
            TraceEvent::Response(msg) => self.write_response(&mut *out, msg),
            TraceEvent::ParseError(error) => self.write_parse_error(&mut *out, error),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use intake_ai::llm::ToolCall;

    struct BufWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for BufWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn tc(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args.to_string(),
        }
    }

    fn render(colors: bool, events: Vec<TraceEvent>) -> String {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let out: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(Box::new(BufWriter(Arc::clone(&buf)))));
        let renderer = TraceRenderer::new(out, colors);
        for event in events {
            renderer.on_event(&event);
        }
        let out = buf.lock().unwrap().clone();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn test_trace_requests_role_prefixed_lines() {
        let out = render(
            false,
            vec![TraceEvent::MessagesSent(&[
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
            ])],
        );
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
    fn test_trace_responses_reasoning_tools_and_output() {
        let msg = AssistantMessage {
            content: Some("final toml".to_string()),
            tool_calls: vec![tc("c1", "usda_search", "{\"queries\":[\"rice\"]}")],
            reasoning: Some("think step".to_string()),
        };
        let out = render(false, vec![TraceEvent::Response(&msg)]);
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
    fn test_trace_parse_error_line() {
        let out = render(false, vec![TraceEvent::ParseError("invalid digit")]);
        assert!(out.contains("[parse error] invalid digit"), "got: {out}");
    }

    #[test]
    fn test_trace_colors_markers_and_lines() {
        let out = render(
            true,
            vec![
                TraceEvent::Response(&AssistantMessage::text("x")),
                TraceEvent::MessagesSent(&[Message::User("x".to_string())]),
                TraceEvent::ParseError("bad"),
            ],
        );
        assert!(out.contains("\x1b[1;33m"), "bold yellow marker: {out}");
        assert!(out.contains("\x1b[32m"), "green response line: {out}");
        assert!(out.contains("\x1b[36m"), "cyan request line: {out}");
        assert!(out.contains("\x1b[31m"), "red parse error: {out}");
        assert!(out.contains("\x1b[0m"), "reset: {out}");
    }
}
