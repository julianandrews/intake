use std::path::PathBuf;
use std::process::Command;

fn foods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foods")
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_intake"))
}

fn run(args: &[&str]) -> (String, bool) {
    let output = Command::new(binary())
        .args(args)
        .output()
        .expect("failed to run intake");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();
    if !success {
        eprintln!("stderr: {}", stderr);
    }
    (stdout, success)
}

fn temp_log_dir(name: &str) -> (String, PathBuf) {
    let dir = std::env::temp_dir().join(format!("intake-test-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    (dir.to_string_lossy().to_string(), dir)
}

fn run_with_log_dir(args: &[&str]) -> (String, bool) {
    let (log_dir_str, dir) = temp_log_dir("run");
    let fd_str = foods_dir().to_string_lossy().to_string();

    let mut all_args = vec!["--foods-dir", &fd_str, "--log-dir", &log_dir_str];
    all_args.extend(args);
    let result = run(&all_args);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn test_list_all_recipes() {
    let (stdout, success) = run_with_log_dir(&["list"]);
    assert!(success);
    assert!(stdout.contains("All Recipes"));
    assert!(stdout.contains("Coffee"));
    assert!(stdout.contains("Oatmeal"));
    assert!(stdout.contains("Turkey Chili 98% Lean"));
}

#[test]
fn test_show_recipe() {
    let (stdout, success) = run_with_log_dir(&["show", "coffee"]);
    assert!(success);
    assert!(stdout.contains("Coffee (1 serving)"));
    assert!(stdout.contains("Cold Brew"));
    assert!(stdout.contains("Oat Milk"));
}

#[test]
fn test_show_recipe_not_found() {
    let (_, success) = run_with_log_dir(&["show", "nonexistent-recipe"]);
    assert!(!success);
}

#[test]
fn test_log_no_entries() {
    let (stdout, success) = run_with_log_dir(&["log", "2026-01-01"]);
    assert!(success);
    assert!(stdout.contains("No entries for 2026-01-01"));
}

#[test]
fn test_add_and_log_workflow() {
    let (log_dir_str, dir) = temp_log_dir("add_and_log");

    let (add_out, add_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "add",
        "coffee",
        "2",
    ]);
    assert!(add_ok, "add failed: {}", add_out);
    assert!(add_out.contains("Coffee"));

    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "log",
    ]);
    assert!(log_ok, "log failed: {}", log_out);
    assert!(log_out.contains("Coffee"));
    assert!(log_out.contains("48")); // 2 servings × 24 cal

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_adhoc_entry() {
    let (log_dir_str, dir) = temp_log_dir("adhoc");

    let (adhoc_out, adhoc_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "adhoc",
        "--calories",
        "250",
        "--protein",
        "12",
        "--fiber",
        "3",
        "Greek yogurt",
        "1.5",
    ]);
    assert!(adhoc_ok, "adhoc failed: {}", adhoc_out);
    assert!(adhoc_out.contains("Greek yogurt"));

    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "log",
    ]);
    assert!(log_ok, "log failed: {}", log_out);
    assert!(log_out.contains("Greek yogurt"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_exercise_recording() {
    let (log_dir_str, dir) = temp_log_dir("exercise");

    let (ex_out, ex_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "exercise",
        "300",
    ]);
    assert!(ex_ok, "exercise failed: {}", ex_out);
    assert!(ex_out.contains("300"));

    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "log",
    ]);
    assert!(log_ok, "log failed: {}", log_out);
    assert!(log_out.contains("300"));
    assert!(log_out.contains("Exercise"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_add_multiple_and_ungrouped() {
    let (log_dir_str, dir) = temp_log_dir("multi");
    let fd_str = foods_dir().to_string_lossy().to_string();

    run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "add",
        "coffee",
        "1",
    ]);
    run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "add",
        "coffee",
        "2",
    ]);
    run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "add",
        "oatmeal",
        "1",
    ]);

    let (grouped, _) = run(&["--foods-dir", &fd_str, "--log-dir", &log_dir_str, "log"]);
    assert!(grouped.contains("Coffee"));
    assert!(grouped.contains("3")); // grouped: 1+2=3 servings

    let (ungrouped, _) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "log",
        "--ungrouped",
    ]);
    assert!(ungrouped.contains("Coffee"));

    let _ = std::fs::remove_dir_all(&dir);
}
