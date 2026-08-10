use intake_ai::confirm::{ConfirmDecision, ConfirmError, Confirmer};
use intake_ai::llm::{AssistantMessage, LlmBackend, LlmError, Message, TraceSink};
use intake_ai::settings::AiSettings;
use intake_ai::tools::Tool;
use serde_json::Value;
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use std::cell::RefCell;
use std::io::{IsTerminal, Write};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Braille dots, the classic CLI spinner; wraps around every 10 frames.
const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const TICK: Duration = Duration::from_millis(80);
const MESSAGE: &str = "Thinking";
const ERASE: &str = "\r\x1b[2K";

fn frame(i: usize) -> &'static str {
    FRAMES[i % FRAMES.len()]
}

/// Whether tracing takes over stderr, so the status line must stay off.
fn trace_on(settings: &AiSettings) -> bool {
    settings.trace_requests || settings.trace_responses
}

/// Whether stderr could carry a spinner line: a real terminal, or the
/// test-forced tty flag. Every place that decides if a line may be on
/// screen checks this, so the fallback paths (plain stderr writes) agree
/// with what the workers actually do.
fn stderr_is_tty() -> bool {
    #[cfg(test)]
    if TEST_TTY.with(|c| c.get()) {
        return true;
    }
    std::io::stderr().is_terminal()
}

fn line(frame: &str, text: &str, secs: u64) -> String {
    format!("{ERASE}{frame} {text} · {secs}s")
}

/// Writes frames to the real stderr. A plain `Stderr` handle could be boxed
/// too, but this keeps the writer construction explicit and `Send`-infallible.
#[cfg(not(test))]
struct StderrSink;

#[cfg(not(test))]
impl Write for StderrSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::io::stderr().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

#[cfg(test)]
thread_local! {
    // These live on the test's thread and are never reset between tests.
    // That is safe because libtest runs every test on its own fresh thread
    // (even with `--test-threads=1`), so no state leaks across tests.
    // Keep it that way — running tests on a shared thread would need resets.
    /// Tests force lines on even though the captured stderr is not a tty.
    static TEST_TTY: Cell<bool> = const { Cell::new(false) };
    /// Tests inject a buffer so frames land somewhere observable instead of
    /// the real stderr (spawned threads bypass libtest's output capture).
    static TEST_OUT: RefCell<Option<Box<dyn Write + Send>>> = const { RefCell::new(None) };
}

#[cfg(not(test))]
fn default_writer() -> Box<dyn Write + Send> {
    Box::new(StderrSink)
}

#[cfg(test)]
fn default_writer() -> Box<dyn Write + Send> {
    TEST_OUT
        .with(|cell| cell.borrow_mut().take())
        .unwrap_or_else(|| Box::new(std::io::sink()))
}

#[cfg(test)]
struct TestSink(Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl TestSink {
    fn new() -> (TestSink, Arc<Mutex<Vec<u8>>>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        (TestSink(Arc::clone(&buf)), buf)
    }
}

#[cfg(test)]
impl Write for TestSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct StatusInner {
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    status: Arc<Mutex<String>>,
    handle: Option<JoinHandle<()>>,
    #[cfg(test)]
    draws: Arc<AtomicUsize>,
    #[cfg(test)]
    active_micros: Arc<AtomicU64>,
}

/// A status line that lives for the whole resolve session instead of per
/// blocking call: the worker thread animates continuously, and callers swap
/// the text (and pause around stderr interaction such as the confirmation
/// prompt) without the line ever blinking off. Only active when stderr is a
/// terminal; every method is a no-op otherwise.
pub(crate) struct StatusLine {
    inner: Option<StatusInner>,
    /// The session's total working time in micros, shared across per-call
    /// lines so the elapsed timer carries from one call to the next instead
    /// of resetting.
    total: Arc<AtomicU64>,
    /// The session's write lock, shared by every line of the session (the
    /// session worker, per-call lines, and the trace sink) so frames, erases,
    /// and trace dumps all serialize on the same writer.
    out: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl StatusLine {
    /// Creates a status line, animated only when stderr is a terminal and
    /// tracing is off (tracing takes over stderr).
    pub(crate) fn new(settings: &AiSettings) -> StatusLine {
        Self::with_tty(!trace_on(settings) && stderr_is_tty())
    }

    fn with_tty(tty: bool) -> StatusLine {
        Self::with_tty_and_clock(tty, Arc::new(AtomicU64::new(0)))
    }

    fn with_tty_and_clock(tty: bool, total: Arc<AtomicU64>) -> StatusLine {
        Self::with_tty_and_clock_and_out(tty, total, Arc::new(Mutex::new(default_writer())))
    }

    fn with_tty_and_clock_and_out(
        tty: bool,
        total: Arc<AtomicU64>,
        out: Arc<Mutex<Box<dyn Write + Send>>>,
    ) -> StatusLine {
        #[cfg(test)]
        let tty = tty || TEST_TTY.with(|c| c.get());
        if !tty {
            return StatusLine {
                inner: None,
                total,
                out,
            };
        }
        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(MESSAGE.to_string()));
        #[cfg(test)]
        let draws = Arc::new(AtomicUsize::new(0));
        #[cfg(test)]
        let active_micros = Arc::new(AtomicU64::new(0));
        let handle = thread::spawn({
            let stop = Arc::clone(&stop);
            let paused = Arc::clone(&paused);
            let status = Arc::clone(&status);
            let out = Arc::clone(&out);
            let total_worker = Arc::clone(&total);
            #[cfg(test)]
            let draws = Arc::clone(&draws);
            #[cfg(test)]
            let active_micros = Arc::clone(&active_micros);
            move || {
                let mut last = std::time::Instant::now();
                let mut active = Duration::from_micros(total_worker.load(Ordering::Relaxed));
                let mut i = 0usize;
                loop {
                    let now = std::time::Instant::now();
                    let delta = now.duration_since(last);
                    last = now;
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    // The draw holds the out lock and re-checks `paused`
                    // under it, so pause() can guarantee no frame lands
                    // after it erases the line.
                    let mut out = out.lock().unwrap_or_else(PoisonError::into_inner);
                    if !paused.load(Ordering::Relaxed) {
                        // Only time spent drawing counts: the seconds shown
                        // are model working time, not prompt time.
                        active += delta;
                        let text = status.lock().unwrap().clone();
                        let secs = active.as_secs();
                        let _ = write!(out, "{}", line(frame(i), &text, secs));
                        let _ = out.flush();
                        total_worker.store(active.as_micros() as u64, Ordering::Relaxed);
                        i += 1;
                        #[cfg(test)]
                        draws.fetch_add(1, Ordering::Relaxed);
                        #[cfg(test)]
                        active_micros.fetch_add(delta.as_micros() as u64, Ordering::Relaxed);
                    }
                    drop(out);
                    thread::sleep(TICK);
                }
            }
        });
        StatusLine {
            inner: Some(StatusInner {
                stop,
                paused,
                status,
                handle: Some(handle),
                #[cfg(test)]
                draws,
                #[cfg(test)]
                active_micros,
            }),
            total,
            out,
        }
    }

    /// Creates a short-lived line for a single blocking call (per-call
    /// mode), seeded from and mirroring back into this line's shared
    /// session total, so the elapsed seconds carry across calls.
    fn per_call_line(&self) -> StatusLine {
        Self::with_tty_and_clock_and_out(
            stderr_is_tty(),
            Arc::clone(&self.total),
            Arc::clone(&self.out),
        )
    }

    /// The session's write lock as a [`TraceSink`], so trace dumps serialize
    /// with frames. Valid even when the line is inactive (per-call mode);
    /// the underlying writer is plain stderr then.
    pub(crate) fn sink(&self) -> Arc<dyn TraceSink> {
        Arc::clone(&self.out) as Arc<dyn TraceSink>
    }

    /// Swaps the status text; the worker redraws the line on its next tick.
    pub(crate) fn set_status(&self, text: impl Into<String>) {
        if let Some(inner) = &self.inner {
            *inner.status.lock().unwrap() = text.into();
        }
    }

    /// Erases the line and freezes the animation — for stderr interaction
    /// (the confirmation prompt). The erase happens synchronously under the
    /// worker's write lock, so no frame can land on the line afterwards,
    /// and while paused the worker writes nothing at all.
    pub(crate) fn pause(&self) {
        if let Some(inner) = &self.inner {
            inner.paused.store(true, Ordering::Relaxed);
            let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = write!(out, "{ERASE}");
            let _ = out.flush();
        }
    }

    /// Resumes the animation after [`StatusLine::pause`].
    pub(crate) fn resume(&self) {
        if let Some(inner) = &self.inner {
            inner.paused.store(false, Ordering::Relaxed);
        }
    }

    /// Prints a line to stderr without the spinner garbling it: the erase
    /// and the message go through the shared writer while it is held, so no
    /// frame can land between them, and the next frame redraws below the
    /// message. When the session line is inactive but stderr is a terminal,
    /// a per-call line may be animating on the same writer, so the message
    /// still goes through it (rather than a raw stderr print, which would
    /// glue the message onto a frame line). Only when nothing can be on
    /// screen (stderr is not a terminal) does it fall back to a plain
    /// stderr print, keeping escape codes out of piped output. While the
    /// line is paused (confirmation prompt), the message prints plainly:
    /// the line is already erased, and an unconditional erase could
    /// clobber whatever the prompt wrote to stderr.
    pub(crate) fn warn(&self, msg: &str) {
        let paused = self
            .inner
            .as_ref()
            .is_some_and(|inner| inner.paused.load(Ordering::Relaxed));
        if paused {
            eprintln!("{msg}");
            return;
        }
        if self.inner.is_some() || stderr_is_tty() {
            let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = writeln!(out, "{ERASE}{msg}");
            let _ = out.flush();
        } else {
            eprintln!("{msg}");
        }
    }

    /// Whether this line is animating (active) at all; callers that need
    /// per-call feedback when it isn't (tracing mode) branch on this.
    pub(crate) fn is_active(&self) -> bool {
        self.inner.is_some()
    }

    fn shutdown(inner: &mut StatusInner, out: &Mutex<Box<dyn Write + Send>>) {
        inner.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = inner.handle.take() {
            let _ = handle.join();
        }
        let mut out = out.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = write!(out, "{ERASE}");
        let _ = out.flush();
    }

    #[cfg(test)]
    fn status(&self) -> String {
        match &self.inner {
            Some(inner) => inner.status.lock().unwrap().clone(),
            None => String::new(),
        }
    }

    #[cfg(test)]
    fn paused(&self) -> bool {
        match &self.inner {
            Some(inner) => inner.paused.load(Ordering::Relaxed),
            None => false,
        }
    }

    #[cfg(test)]
    fn draws(&self) -> usize {
        match &self.inner {
            Some(inner) => inner.draws.load(Ordering::Relaxed),
            None => 0,
        }
    }

    #[cfg(test)]
    fn active_micros(&self) -> u64 {
        match &self.inner {
            Some(inner) => inner.active_micros.load(Ordering::Relaxed),
            None => 0,
        }
    }

    #[cfg(test)]
    fn total_micros(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
}

impl Drop for StatusLine {
    fn drop(&mut self) {
        if let Some(inner) = &mut self.inner {
            StatusLine::shutdown(inner, &self.out);
        }
    }
}

fn tool_label(name: &str) -> String {
    match name {
        "usda_search" => "Searching USDA".to_string(),
        "usda_get" => "Fetching USDA nutrition".to_string(),
        "food_lookup" => "Looking up your foods".to_string(),
        other => format!("Running {other}"),
    }
}

/// Wraps a [`Tool`] to show the tool's friendly label while it runs. In
/// session mode it swaps the session line's text; when the session line is
/// inactive (tracing mode), a temporary per-call line animates for the
/// duration of the call instead, so the label never goes missing. Either
/// way the elapsed timer continues from the shared session total.
pub(crate) struct StatusTool<'a> {
    inner: &'a dyn Tool,
    status: &'a StatusLine,
}

impl<'a> StatusTool<'a> {
    pub(crate) fn new(inner: &'a dyn Tool, status: &'a StatusLine) -> StatusTool<'a> {
        StatusTool { inner, status }
    }
}

impl Tool for StatusTool<'_> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> Value {
        self.inner.schema()
    }

    fn execute(&self, params: &Value) -> Result<String, String> {
        let label = tool_label(self.inner.name());
        if self.status.is_active() {
            self.status.set_status(label);
            let result = self.inner.execute(params);
            self.status.set_status(MESSAGE);
            result
        } else {
            let line = self.status.per_call_line();
            line.set_status(label);
            let result = self.inner.execute(params);
            drop(line);
            result
        }
    }
}

/// Wraps a [`Confirmer`] to park the status line before the confirmation
/// prompt writes to stderr; the next model round resumes it.
pub(crate) struct SpinnerConfirmer<'a> {
    inner: Box<dyn Confirmer + 'a>,
    status: &'a StatusLine,
}

impl<'a> SpinnerConfirmer<'a> {
    pub(crate) fn new(
        inner: Box<dyn Confirmer + 'a>,
        status: &'a StatusLine,
    ) -> SpinnerConfirmer<'a> {
        SpinnerConfirmer { inner, status }
    }
}

impl Confirmer for SpinnerConfirmer<'_> {
    fn confirm(&mut self, rendered: &str) -> Result<ConfirmDecision, ConfirmError> {
        self.status.pause();
        self.inner.confirm(rendered)
    }

    fn present_before_confirm(&self) -> bool {
        self.inner.present_before_confirm()
    }
}

/// Wraps an [`LlmBackend`] with progress feedback during `complete` calls.
///
/// Two modes, decided once from the settings:
/// * tracing on — a per-call [`StatusLine`] animates only while the main
///   thread is blocked, and `drop` joins the worker before returning, so it
///   is safe alongside trace dumps on stderr. The per-call lines share the
///   session's total elapsed, so the timer carries across calls;
/// * tracing off — the [`StatusLine`] keeps animating across rounds and
///   retries; each call resumes it and swaps in the model label, so the
///   line updates instead of blinking off and on.
pub(crate) struct SpinnerBackend<'a> {
    inner: &'a dyn LlmBackend,
    status: &'a StatusLine,
    per_call: bool,
}

impl<'a> SpinnerBackend<'a> {
    pub(crate) fn new(
        inner: &'a dyn LlmBackend,
        settings: &AiSettings,
        status: &'a StatusLine,
    ) -> SpinnerBackend<'a> {
        SpinnerBackend {
            inner,
            status,
            per_call: trace_on(settings),
        }
    }
}

impl LlmBackend for SpinnerBackend<'_> {
    fn complete(
        &self,
        messages: &[Message],
        tools: &[&dyn Tool],
    ) -> Result<AssistantMessage, LlmError> {
        if self.per_call {
            let status = self.status.per_call_line();
            let result = self.inner.complete(messages, tools);
            drop(status);
            result
        } else {
            // While paused the worker draws nothing, so the label can be
            // swapped before resuming — the next tick draws the new text,
            // never a stale tool label.
            self.status.set_status(MESSAGE);
            self.status.resume();
            self.inner.complete(messages, tools)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use intake_ai::pipeline::{ResolveContext, Resolver};
    use std::sync::Mutex;

    struct FakeBackend {
        calls: Mutex<usize>,
        result: Mutex<Result<AssistantMessage, LlmError>>,
        gate: Option<Arc<AtomicBool>>,
    }

    impl FakeBackend {
        fn new(result: Result<AssistantMessage, LlmError>) -> FakeBackend {
            FakeBackend {
                calls: Mutex::new(0),
                result: Mutex::new(result),
                gate: None,
            }
        }

        fn gated(result: Result<AssistantMessage, LlmError>, gate: Arc<AtomicBool>) -> FakeBackend {
            FakeBackend {
                calls: Mutex::new(0),
                result: Mutex::new(result),
                gate: Some(gate),
            }
        }

        fn calls(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }

    impl LlmBackend for FakeBackend {
        fn complete(
            &self,
            _messages: &[Message],
            _tools: &[&dyn Tool],
        ) -> Result<AssistantMessage, LlmError> {
            if let Some(gate) = &self.gate {
                while !gate.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            *self.calls.lock().unwrap() += 1;
            self.result.lock().unwrap().clone()
        }
    }

    struct FakeTool {
        name: &'static str,
        fail: bool,
        gate: Option<Arc<AtomicBool>>,
    }

    impl FakeTool {
        fn new(name: &'static str) -> FakeTool {
            FakeTool {
                name,
                fail: false,
                gate: None,
            }
        }

        fn failing(name: &'static str) -> FakeTool {
            FakeTool {
                name,
                fail: true,
                gate: None,
            }
        }

        fn gated(name: &'static str, gate: Arc<AtomicBool>) -> FakeTool {
            FakeTool {
                name,
                fail: false,
                gate: Some(gate),
            }
        }
    }

    impl Tool for FakeTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "fake tool"
        }

        fn schema(&self) -> Value {
            serde_json::json!({})
        }

        fn execute(&self, _params: &Value) -> Result<String, String> {
            if let Some(gate) = &self.gate {
                while !gate.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            if self.fail {
                Err("boom".to_string())
            } else {
                Ok("out".to_string())
            }
        }
    }

    struct FakeConfirmer {
        calls: Mutex<usize>,
        present: bool,
        fail: bool,
    }

    impl FakeConfirmer {
        fn new(present: bool) -> FakeConfirmer {
            FakeConfirmer {
                calls: Mutex::new(0),
                present,
                fail: false,
            }
        }

        fn failing(present: bool) -> FakeConfirmer {
            FakeConfirmer {
                calls: Mutex::new(0),
                present,
                fail: true,
            }
        }
    }

    impl Confirmer for FakeConfirmer {
        fn confirm(&mut self, _rendered: &str) -> Result<ConfirmDecision, ConfirmError> {
            *self.calls.lock().unwrap() += 1;
            if self.fail {
                Err(ConfirmError::Cancelled)
            } else {
                Ok(ConfirmDecision::Accept)
            }
        }

        fn present_before_confirm(&self) -> bool {
            self.present
        }
    }

    fn settings() -> AiSettings {
        AiSettings::default()
    }

    /// An active line whose frames land in an observable buffer.
    fn with_test_line() -> (StatusLine, Arc<Mutex<Vec<u8>>>) {
        let (writer, buf) = TestSink::new();
        TEST_TTY.with(|c| c.set(true));
        TEST_OUT.with(|cell| *cell.borrow_mut() = Some(Box::new(writer)));
        (StatusLine::with_tty(false), buf)
    }

    fn resolve_with(
        backend: &dyn LlmBackend,
        confirmer: &mut dyn Confirmer,
    ) -> Result<i64, intake_ai::pipeline::ResolveError> {
        let s = settings();
        let ctx = ResolveContext {
            settings: &s,
            backend,
            tools: &[],
            trace_sink: None,
        };
        let mut resolver = Resolver::new(&ctx, confirmer, "system".to_string());
        resolver.resolve(
            "x",
            |s: &str| s.trim().parse::<i64>().map_err(|e| e.to_string()),
            &|v: &i64| format!("proposal: {v}"),
        )
    }

    #[test]
    fn test_frame_cycle_wraps() {
        assert_eq!(frame(0), "⠋");
        assert_eq!(frame(9), "⠏");
        assert_eq!(frame(10), frame(0));
        assert_eq!(frame(37), frame(7));
    }

    #[test]
    fn test_trace_on_flag() {
        assert!(!trace_on(&settings()));
        let mut requests = settings();
        requests.trace_requests = true;
        assert!(trace_on(&requests));
        let mut responses = settings();
        responses.trace_responses = true;
        assert!(trace_on(&responses));
    }

    #[test]
    fn test_status_line_elapsed_format() {
        assert_eq!(
            line("⠋", "Searching USDA", 14),
            "\r\x1b[2K⠋ Searching USDA · 14s"
        );
    }

    #[test]
    fn test_status_line_inactive_noops() {
        let status = StatusLine::with_tty(false);
        assert!(!status.is_active());
        status.set_status("x");
        status.pause();
        status.resume();
        assert_eq!(status.status(), "");
        assert!(!status.paused());
    }

    #[test]
    fn test_status_line_active_lifecycle() {
        let status = StatusLine::with_tty(true);
        assert!(status.is_active());
        assert_eq!(status.status(), MESSAGE);
        status.set_status("Searching USDA");
        assert_eq!(status.status(), "Searching USDA");
        status.pause();
        assert!(status.paused());
        status.resume();
        assert!(!status.paused());
        status.set_status("Thinking");
        assert_eq!(status.status(), "Thinking");
    }

    #[test]
    fn test_status_line_drop_stops_thread() {
        drop(StatusLine::with_tty(true));
    }

    #[test]
    fn test_frames_go_to_writer_not_stderr() {
        let (status, buf) = with_test_line();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while buf.lock().unwrap().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "no frame appeared in the writer"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        assert!(text.contains(MESSAGE));
        drop(status);
        let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        assert!(text.ends_with(ERASE), "line erased on drop");
    }

    #[test]
    fn test_paused_line_writes_nothing() {
        let status = StatusLine::with_tty(true);
        status.pause();
        let before = status.draws();
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(
            status.draws(),
            before,
            "no frames may land while paused — the prompt line must stay put"
        );
        status.resume();
        std::thread::sleep(Duration::from_millis(250));
        assert!(status.draws() > before, "frames resume after unpausing");
    }

    #[test]
    fn test_paused_line_writes_nothing_to_writer() {
        let (status, buf) = with_test_line();
        status.pause();
        let n = buf.lock().unwrap().len();
        thread::sleep(Duration::from_millis(250));
        assert_eq!(
            buf.lock().unwrap().len(),
            n,
            "no bytes may land while paused — the prompt line must stay put"
        );
        status.resume();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while buf.lock().unwrap().len() == n {
            assert!(
                std::time::Instant::now() < deadline,
                "frames resume after unpausing"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn test_warn_erases_frame_and_prints_message() {
        let (status, buf) = with_test_line();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while buf.lock().unwrap().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "no frame appeared in the writer"
            );
            thread::sleep(Duration::from_millis(10));
        }
        status.warn("Warning: skipped broken.toml");
        let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        assert!(
            text.contains(&format!("{ERASE}Warning: skipped broken.toml\n")),
            "erase and message must be written contiguously: {text:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
        let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        assert!(
            text.contains("Warning: skipped broken.toml\n"),
            "frames must redraw below the message, not wipe it: {text:?}"
        );
        drop(status);
    }

    #[test]
    fn test_warn_inactive_line_prints_and_stays_inactive() {
        let status = StatusLine::with_tty(false);
        status.warn("hello");
        assert!(!status.is_active());
    }

    #[test]
    fn test_warn_while_per_call_line_active_erases_and_keeps_message() {
        let (writer, buf) = TestSink::new();
        TEST_OUT.with(|cell| *cell.borrow_mut() = Some(Box::new(writer)));
        let status = StatusLine::with_tty(false);
        TEST_TTY.with(|c| c.set(true));
        let per_call = status.per_call_line();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while buf.lock().unwrap().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "per-call line never drew"
            );
            thread::sleep(Duration::from_millis(10));
        }
        status.warn("Warning: skipped broken.toml");
        let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        assert!(
            text.contains(&format!("{ERASE}Warning: skipped broken.toml\n")),
            "erase and message must be written contiguously: {text:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
        let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        assert!(
            text.contains("Warning: skipped broken.toml\n"),
            "frames must redraw below the message, not wipe it: {text:?}"
        );
        drop(per_call);
    }

    #[test]
    fn test_paused_time_not_counted_in_elapsed() {
        let status = StatusLine::with_tty(true);
        status.pause();
        std::thread::sleep(Duration::from_millis(300));
        let frozen = status.active_micros();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            status.active_micros(),
            frozen,
            "no time accumulates while paused — seconds are model working time"
        );
        status.resume();
        std::thread::sleep(Duration::from_millis(250));
        assert!(
            status.active_micros() > frozen,
            "active time resumes accumulating"
        );
    }

    #[test]
    fn test_tool_label_mapping() {
        assert_eq!(tool_label("usda_search"), "Searching USDA");
        assert_eq!(tool_label("usda_get"), "Fetching USDA nutrition");
        assert_eq!(tool_label("food_lookup"), "Looking up your foods");
        assert_eq!(tool_label("unknown_tool"), "Running unknown_tool");
    }

    #[test]
    fn test_status_tool_delegates_and_resets_label() {
        let tool = FakeTool::new("usda_search");
        let status = StatusLine::with_tty(true);
        let wrapped = StatusTool::new(&tool, &status);
        assert_eq!(wrapped.name(), "usda_search");
        assert_eq!(wrapped.description(), "fake tool");
        assert_eq!(wrapped.schema(), serde_json::json!({}));
        assert_eq!(wrapped.execute(&serde_json::json!({})).unwrap(), "out");
        assert_eq!(status.status(), MESSAGE);
    }

    #[test]
    fn test_status_tool_sets_label_during_run() {
        let gate = Arc::new(AtomicBool::new(false));
        let tool = FakeTool::gated("usda_search", Arc::clone(&gate));
        let status = StatusLine::with_tty(true);
        let status_arc = Arc::clone(&status.inner.as_ref().unwrap().status);
        let gate_arc = Arc::clone(&gate);
        let wrapped = StatusTool::new(&tool, &status);
        thread::scope(|s| {
            let observer = s.spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while *status_arc.lock().unwrap() != "Searching USDA" {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "tool label never appeared while the tool ran"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                gate_arc.store(true, Ordering::Relaxed);
            });
            wrapped.execute(&serde_json::json!({})).unwrap();
            observer.join().unwrap();
        });
        assert_eq!(status.status(), MESSAGE);
    }

    #[test]
    fn test_status_tool_error_propagates_and_resets_label() {
        let tool = FakeTool::failing("usda_get");
        let status = StatusLine::with_tty(true);
        let wrapped = StatusTool::new(&tool, &status);
        let err = wrapped.execute(&serde_json::json!({})).unwrap_err();
        assert_eq!(err, "boom");
        assert_eq!(status.status(), MESSAGE);
    }

    #[test]
    fn test_status_tool_per_call_mode_uses_temp_line() {
        let gate = Arc::new(AtomicBool::new(false));
        let tool = FakeTool::gated("usda_search", Arc::clone(&gate));
        let (writer, buf) = TestSink::new();
        TEST_OUT.with(|cell| *cell.borrow_mut() = Some(Box::new(writer)));
        let status = StatusLine::with_tty(false);
        TEST_TTY.with(|c| c.set(true));
        let wrapped = StatusTool::new(&tool, &status);
        thread::scope(|s| {
            let buf = Arc::clone(&buf);
            let gate = Arc::clone(&gate);
            let observer = s.spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                loop {
                    let bytes = buf.lock().unwrap().clone();
                    let found = bytes
                        .windows(b"Searching USDA".len())
                        .any(|w| w == b"Searching USDA");
                    if found {
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "tool label never appeared in the per-call line"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                gate.store(true, Ordering::Relaxed);
            });
            wrapped.execute(&serde_json::json!({})).unwrap();
            observer.join().unwrap();
        });
        assert!(
            !status.is_active(),
            "session line stays parked in per-call mode"
        );
        let bytes = buf.lock().unwrap().clone();
        assert!(
            bytes.ends_with(ERASE.as_bytes()),
            "per-call line erased on drop"
        );
    }

    #[test]
    fn test_confirmer_pauses_and_delegates() {
        let confirmer = FakeConfirmer::new(true);
        let status = StatusLine::with_tty(true);
        let mut wrapped = SpinnerConfirmer::new(Box::new(confirmer), &status);
        assert!(wrapped.present_before_confirm());
        assert!(matches!(
            wrapped.confirm("proposal").unwrap(),
            ConfirmDecision::Accept
        ));
        assert!(status.paused());
        assert_eq!(status.status(), MESSAGE);
    }

    #[test]
    fn test_confirmer_error_propagates_and_line_stays_parked() {
        let confirmer = FakeConfirmer::failing(true);
        let status = StatusLine::with_tty(true);
        let mut wrapped = SpinnerConfirmer::new(Box::new(confirmer), &status);
        assert!(matches!(
            wrapped.confirm("proposal").unwrap_err(),
            ConfirmError::Cancelled
        ));
        assert!(
            status.paused(),
            "no resume without a further model round — drop cleans up"
        );
    }

    #[test]
    fn test_confirmer_delegates_present() {
        let confirmer = FakeConfirmer::new(false);
        let status = StatusLine::with_tty(true);
        let wrapped = SpinnerConfirmer::new(Box::new(confirmer), &status);
        assert!(!wrapped.present_before_confirm());
    }

    #[test]
    fn test_resolve_accept_parks_line_after_final_confirm() {
        let backend = FakeBackend::new(Ok(AssistantMessage::text("42")));
        let status = StatusLine::with_tty(true);
        let confirmer = FakeConfirmer::new(true);
        let mut wrapped = SpinnerConfirmer::new(Box::new(confirmer), &status);
        let value = resolve_with(&backend, &mut wrapped).unwrap();
        assert_eq!(value, 42);
        assert!(
            status.paused(),
            "line parked after the final accept, erased on drop"
        );
    }

    #[test]
    fn test_resolve_no_present_never_pauses() {
        let backend = FakeBackend::new(Ok(AssistantMessage::text("7")));
        let status = StatusLine::with_tty(true);
        let confirmer = FakeConfirmer::new(false);
        let mut wrapped = SpinnerConfirmer::new(Box::new(confirmer), &status);
        let value = resolve_with(&backend, &mut wrapped).unwrap();
        assert_eq!(value, 7);
        assert!(!status.paused());
    }

    #[test]
    fn test_backend_session_mode_sets_status_and_delegates() {
        let backend = FakeBackend::new(Ok(AssistantMessage::text("hi")));
        let status = StatusLine::with_tty(true);
        let wrapper = SpinnerBackend::new(&backend, &settings(), &status);
        assert!(!wrapper.per_call);
        let msg = wrapper
            .complete(&[Message::User("x".to_string())], &[])
            .unwrap();
        assert_eq!(msg, AssistantMessage::text("hi"));
        assert_eq!(backend.calls(), 1);
        assert_eq!(status.status(), MESSAGE);
        assert!(!status.paused());
    }

    #[test]
    fn test_backend_session_mode_error_propagates() {
        let backend = FakeBackend::new(Err(LlmError::Timeout));
        let status = StatusLine::with_tty(true);
        let wrapper = SpinnerBackend::new(&backend, &settings(), &status);
        let err = wrapper.complete(&[], &[]).unwrap_err();
        assert!(matches!(err, LlmError::Timeout));
        assert_eq!(backend.calls(), 1);
        assert_eq!(status.status(), MESSAGE);
    }

    #[test]
    fn test_backend_per_call_mode_with_trace_keeps_status_line() {
        let backend = FakeBackend::new(Ok(AssistantMessage::text("hi")));
        let status = StatusLine::with_tty(true);
        let mut s = settings();
        s.trace_requests = true;
        let wrapper = SpinnerBackend::new(&backend, &s, &status);
        assert!(wrapper.per_call);
        let msg = wrapper
            .complete(&[Message::User("x".to_string())], &[])
            .unwrap();
        assert_eq!(msg, AssistantMessage::text("hi"));
        assert_eq!(backend.calls(), 1);
        assert_eq!(
            status.status(),
            MESSAGE,
            "session line untouched in per-call mode"
        );
        assert!(!status.paused());
    }

    #[test]
    fn test_line_seeds_elapsed_from_clock() {
        let (writer, buf) = TestSink::new();
        TEST_TTY.with(|c| c.set(true));
        TEST_OUT.with(|cell| *cell.borrow_mut() = Some(Box::new(writer)));
        let total = Arc::new(AtomicU64::new(1_100_000));
        let status = StatusLine::with_tty_and_clock(false, total);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
            if text.contains("· 1s") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "seeded elapsed never appeared in the frames"
            );
            thread::sleep(Duration::from_millis(10));
        }
        drop(status);
    }

    #[test]
    fn test_per_call_lines_share_session_total() {
        let (writer, _buf) = TestSink::new();
        TEST_OUT.with(|cell| *cell.borrow_mut() = Some(Box::new(writer)));
        let status = StatusLine::with_tty(false);
        TEST_TTY.with(|c| c.set(true));
        let line1 = status.per_call_line();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while status.total_micros() == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "first per-call line never drew"
            );
            thread::sleep(Duration::from_millis(10));
        }
        drop(line1);
        let after1 = status.total_micros();
        assert!(after1 > 0, "first line must have counted working time");
        let line2 = status.per_call_line();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while status.total_micros() <= after1 {
            assert!(
                std::time::Instant::now() < deadline,
                "second line did not continue the timer from the shared total"
            );
            thread::sleep(Duration::from_millis(10));
        }
        drop(line2);
        assert!(
            status.total_micros() > after1,
            "elapsed must carry across per-call lines"
        );
    }

    #[test]
    fn test_backend_per_call_elapsed_carries_across_calls() {
        let (writer, _buf) = TestSink::new();
        TEST_OUT.with(|cell| *cell.borrow_mut() = Some(Box::new(writer)));
        let status = StatusLine::with_tty(false);
        TEST_TTY.with(|c| c.set(true));
        let mut s = settings();
        s.trace_requests = true;

        let gate1 = Arc::new(AtomicBool::new(false));
        let backend1 = FakeBackend::gated(Ok(AssistantMessage::text("a")), Arc::clone(&gate1));
        let wrapper1 = SpinnerBackend::new(&backend1, &s, &status);
        thread::scope(|sc| {
            let release = sc.spawn(|| {
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while status.total_micros() == 0 && std::time::Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                gate1.store(true, Ordering::Relaxed);
                assert!(status.total_micros() > 0, "no frame from the first call");
            });
            wrapper1
                .complete(&[Message::User("x".to_string())], &[])
                .unwrap();
            release.join().unwrap();
        });
        let first = status.total_micros();

        let gate2 = Arc::new(AtomicBool::new(false));
        let backend2 = FakeBackend::gated(Ok(AssistantMessage::text("b")), Arc::clone(&gate2));
        let wrapper2 = SpinnerBackend::new(&backend2, &s, &status);
        thread::scope(|sc| {
            let release = sc.spawn(|| {
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while status.total_micros() < first + 100_000
                    && std::time::Instant::now() < deadline
                {
                    thread::sleep(Duration::from_millis(10));
                }
                gate2.store(true, Ordering::Relaxed);
                assert!(
                    status.total_micros() > first,
                    "second call did not continue the timer"
                );
            });
            wrapper2
                .complete(&[Message::User("y".to_string())], &[])
                .unwrap();
            release.join().unwrap();
        });
        let second = status.total_micros();
        assert!(second > first, "elapsed must carry across complete calls");
    }

    #[test]
    fn test_sink_writes_serialize_with_frames() {
        let (status, buf) = with_test_line();
        let sink = status.sink();
        // Simulate a trace dump that takes a while: the worker must not draw
        // between its lines, or the erase would land mid-dump.
        sink.with_writer(&mut |w| {
            let _ = writeln!(w, "--- to model ---");
            std::thread::sleep(Duration::from_millis(200));
            let _ = writeln!(w, "--- end to model ---");
            Ok(())
        })
        .unwrap();
        let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        let start = text
            .find("--- to model ---")
            .expect("dump start must reach the shared writer");
        let end = text
            .find("--- end to model ---")
            .expect("dump end must reach the shared writer");
        let between = &text[start..end + "--- end to model ---".len()];
        assert!(
            !between.contains("\x1b[2K"),
            "no frame may land inside the dump: {between:?}"
        );
        drop(status);
    }
}
