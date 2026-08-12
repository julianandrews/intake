//! End-to-end tests for the `ai` commands, driven against a fake
//! OpenAI-compatible server — no network or API keys needed. Compiled only
//! when the `ai` feature is on (see `[[test]] required-features` in
//! Cargo.toml).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_intake"))
}

fn foods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foods")
}

fn run_ai(args: &[&str], config_dir: &Path, stdin: Option<&str>) -> (String, String, bool) {
    let mut cmd = Command::new(binary());
    cmd.args(args)
        .env("XDG_CONFIG_HOME", config_dir)
        .env("NO_COLOR", "1");
    let output = match stdin {
        Some(input) => {
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = cmd.spawn().expect("failed to spawn intake");
            child
                .stdin
                .take()
                .expect("piped stdin")
                .write_all(input.as_bytes())
                .expect("failed to write stdin");
            child.wait_with_output().expect("failed to wait for intake")
        }
        None => {
            cmd.stdin(Stdio::null());
            cmd.output().expect("failed to run intake")
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        eprintln!("stderr: {stderr}");
    }
    (stdout, stderr, output.status.success())
}

fn write_day_log(dir: &Path, date: &str, with_coffee: bool) {
    let entries = if with_coffee {
        "[[entries]]\nservings = 1.0\ncalories = 12\nprotein_g = 1.0\nfiber_g = 0.0\nfat_g = 0.0\ncarbs_g = 0.0\nalcohol_g = 0.0\ntitle = \"Coffee\"\n"
    } else {
        ""
    };
    let content = format!("exercise_calories = 0\n\n{entries}");
    std::fs::write(dir.join(format!("{date}.toml")), content).unwrap();
}

/// A minimal OpenAI-compatible `/v1/chat/completions` server. Serves one
/// assistant message per HTTP request, in order, and records the raw
/// request bodies for assertion. The thread is detached; a test that
/// forgets to hit the server fails on the recorded-request assertions.
struct FakeLlm {
    address: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeLlm {
    fn start(contents: &[&'static str]) -> FakeLlm {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_clone = Arc::clone(&requests);
        let contents = contents.to_vec();
        thread::spawn(move || {
            for content in contents {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_full_request(&mut stream);
                requests_clone
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request).into_owned());
                let payload = format!(
                    r#"{{"choices":[{{"message":{{"role":"assistant","content":{content:?}}}}}]}}"#
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    payload.len(),
                    payload
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        FakeLlm { address, requests }
    }

    fn config(&self, config_dir: &Path) {
        std::fs::create_dir_all(config_dir.join("intake")).unwrap();
        std::fs::write(
            config_dir.join("intake").join("config.toml"),
            format!("[ai]\nbase_url = \"{}\"\nmodel = \"fake\"\n", self.address),
        )
        .unwrap();
    }

    fn request_bodies(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// Read a full HTTP request off the wire: the header block plus exactly
/// `Content-Length` body bytes, since a request may arrive in several TCP
/// segments.
fn read_full_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    let header_end;
    let content_length;
    loop {
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0, "connection closed mid-request");
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_header_end(&buf) {
            header_end = pos;
            let head = String::from_utf8_lossy(&buf[..pos]);
            content_length = head
                .lines()
                .find_map(|l| {
                    l.strip_prefix("content-length:")
                        .or_else(|| l.strip_prefix("Content-Length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            break;
        }
    }
    let expected = header_end + content_length;
    while buf.len() < expected {
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0, "connection closed mid-body");
        buf.extend_from_slice(&chunk[..n]);
    }
    buf
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

const ADD_OATMEAL_OPS: &str = "[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 1\n";

/// Per-serving macros for the oatmeal fixture: 418 cal, 22 protein, 9
/// fiber, 12.2 fat, 56.5 carbs, 0 alcohol.
const OATMEAL_LINE: &str = "Oatmeal | 1 | 418, 22, 9, 12.2, 56.5, 0";

#[test]
fn test_ai_log_confirmed_proposal_writes_day() {
    let fake = FakeLlm::start(&[ADD_OATMEAL_OPS]);
    let config_dir = tempfile::TempDir::new().unwrap();
    fake.config(config_dir.path());
    let log_dir = tempfile::TempDir::new().unwrap();
    write_day_log(log_dir.path(), "2026-08-10", true);
    let foods = foods_dir().to_string_lossy().to_string();
    let logs = log_dir.path().to_string_lossy().to_string();

    let (stdout, _, success) = run_ai(
        &[
            "--foods-dir",
            &foods,
            "--log-dir",
            &logs,
            "ai",
            "log",
            "add oatmeal",
            "--date",
            "2026-08-10",
        ],
        config_dir.path(),
        Some("y\n"),
    );
    assert!(success, "stdout: {stdout}");
    assert!(
        stdout.contains(&format!("+ {OATMEAL_LINE}")),
        "proposal diff missing: {stdout}"
    );
    assert!(stdout.contains(OATMEAL_LINE), "day table missing: {stdout}");
    assert!(
        stdout.contains("Logged to 2026-08-10"),
        "confirmation line missing: {stdout}"
    );
    assert_eq!(
        stdout.matches("Total").count(),
        1,
        "day table must render only once: {stdout}"
    );
    let day = std::fs::read_to_string(log_dir.path().join("2026-08-10.toml")).unwrap();
    assert!(day.contains("title = \"Oatmeal\""), "day: {day}");
    assert!(day.contains("servings = 1"), "day: {day}");
    // The added entry is stamped at write time (RFC 3339 UTC); the
    // pre-existing coffee entry keeps no timestamp.
    assert_eq!(
        day.matches("timestamp = \"").count(),
        1,
        "exactly the added entry must carry a timestamp: {day}"
    );
    assert!(
        day.contains("Z\""),
        "stamp must be a UTC RFC 3339 string: {day}"
    );
    assert!(
        day.contains("timestamp = \"20"),
        "stamp must be a plausible year-20xx date: {day}"
    );

    let bodies = fake.request_bodies();
    assert_eq!(bodies.len(), 1, "expected exactly one LLM request");
    assert!(
        bodies[0].contains("\"model\":\"fake\""),
        "body: {}",
        bodies[0]
    );
    assert!(
        bodies[0].contains("food_lookup"),
        "request must advertise the food_lookup tool: {}",
        bodies[0]
    );
    assert!(
        bodies[0].contains("add oatmeal"),
        "request must carry the prompt: {}",
        bodies[0]
    );
}

#[test]
fn test_ai_log_rejected_proposal_writes_nothing() {
    let fake = FakeLlm::start(&[ADD_OATMEAL_OPS]);
    let config_dir = tempfile::TempDir::new().unwrap();
    fake.config(config_dir.path());
    let log_dir = tempfile::TempDir::new().unwrap();
    write_day_log(log_dir.path(), "2026-08-10", true);
    let foods = foods_dir().to_string_lossy().to_string();
    let logs = log_dir.path().to_string_lossy().to_string();

    let (stdout, _, success) = run_ai(
        &[
            "--foods-dir",
            &foods,
            "--log-dir",
            &logs,
            "ai",
            "log",
            "add oatmeal",
            "--date",
            "2026-08-10",
        ],
        config_dir.path(),
        Some("n\n"),
    );
    assert!(success, "stdout: {stdout}");
    assert!(stdout.contains("Nothing written"), "stdout: {stdout}");
    let day = std::fs::read_to_string(log_dir.path().join("2026-08-10.toml")).unwrap();
    assert!(!day.contains("Oatmeal"), "day must be unchanged: {day}");
}

#[test]
fn test_ai_log_yes_skips_confirmation_and_writes() {
    let fake = FakeLlm::start(&[ADD_OATMEAL_OPS]);
    let config_dir = tempfile::TempDir::new().unwrap();
    fake.config(config_dir.path());
    let log_dir = tempfile::TempDir::new().unwrap();
    write_day_log(log_dir.path(), "2026-08-10", true);
    let foods = foods_dir().to_string_lossy().to_string();
    let logs = log_dir.path().to_string_lossy().to_string();

    let (stdout, _, success) = run_ai(
        &[
            "--foods-dir",
            &foods,
            "--log-dir",
            &logs,
            "ai",
            "log",
            "add oatmeal",
            "--date",
            "2026-08-10",
            "--yes",
        ],
        config_dir.path(),
        None,
    );
    assert!(success, "stdout: {stdout}");
    assert!(
        stdout.contains("Total"),
        "day table missing after --yes: {stdout}"
    );
    assert!(
        !stdout.contains("Logged to"),
        "--yes must not print the confirmation line: {stdout}"
    );
    let day = std::fs::read_to_string(log_dir.path().join("2026-08-10.toml")).unwrap();
    assert!(day.contains("title = \"Oatmeal\""), "day: {day}");
}

#[test]
fn test_ai_food_new_writes_food_file() {
    let toml = "title = \"E2e Food\"\nservings = 2\n\n[[ingredients]]\nname = \"X\"\ncalories = 100\nprotein_g = 10\nfiber_g = 1\nfat_g = 2\ncarbs_g = 3\nalcohol_g = 0\n";
    let fake = FakeLlm::start(&[toml]);
    let config_dir = tempfile::TempDir::new().unwrap();
    fake.config(config_dir.path());
    let foods = tempfile::TempDir::new().unwrap();
    let foods_str = foods.path().to_string_lossy().to_string();

    let (stdout, _, success) = run_ai(
        &[
            "--foods-dir",
            &foods_str,
            "ai",
            "food",
            "new",
            "e2e-food",
            "make it",
            "--yes",
        ],
        config_dir.path(),
        None,
    );
    assert!(success, "stdout: {stdout}");
    assert!(
        foods.path().join("e2e-food.toml").exists(),
        "food file was not written"
    );
    assert!(
        !stdout.contains("Wrote"),
        "--yes must not print the confirmation line: {stdout}"
    );
    assert!(stdout.contains("E2e Food"), "stdout: {stdout}");
}
