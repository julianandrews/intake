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

fn run_with_log_dir(args: &[&str]) -> (String, bool) {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let mut all_args = vec!["--foods-dir", &fd_str, "--log-dir", &log_dir_str];
    all_args.extend(args);
    run(&all_args)
}

fn write_day_log(
    dir: &std::path::Path,
    date: &str,
    calories: u32,
    protein: f64,
    fiber: f64,
    exercise: u32,
) {
    let content = format!(
        "exercise_calories = {exercise}\n\n[[entries]]\nslug = \"coffee\"\nservings = 1.0\ncalories = {calories}\nprotein_g = {protein}\nfiber_g = {fiber}\ntitle = \"Coffee\"\n"
    );
    std::fs::write(dir.join(format!("{date}.toml")), content).unwrap();
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
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();

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
}

#[test]
fn test_adhoc_entry() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();

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
}

#[test]
fn test_exercise_recording() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();

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
}

#[test]
fn test_add_multiple_and_grouped() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
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

    // default: ungrouped — each entry is its own row
    let (ungrouped, _) = run(&["--foods-dir", &fd_str, "--log-dir", &log_dir_str, "log"]);
    assert!(ungrouped.contains("Coffee"));

    // --grouped merges entries with the same slug
    let (grouped, _) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "log",
        "--grouped",
    ]);
    assert!(grouped.contains("Coffee"));
    assert!(grouped.contains("3")); // grouped: 1+2=3 servings
}

#[test]
fn test_summary_multi_day() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    // three logged days within a 7-day window; 08-02 also has exercise
    write_day_log(dir.path(), "2026-07-30", 200, 10.0, 4.0, 0);
    write_day_log(dir.path(), "2026-08-01", 300, 20.0, 8.0, 0);
    write_day_log(dir.path(), "2026-08-02", 1000, 50.0, 15.0, 300);

    let (stdout, success) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "summary",
        "2026-08-03",
        "--days",
        "7",
    ]);
    assert!(success, "summary failed: {}", stdout);
    assert!(stdout.contains("Summary 2026-07-30 to 2026-08-02"));
    assert!(stdout.contains("2026-07-30"));
    assert!(stdout.contains("2026-08-01"));
    assert!(stdout.contains("2026-08-02"));
    // unlogged days in the window are skipped
    assert!(!stdout.contains("2026-07-31"));
    assert!(!stdout.contains("2026-08-03"));
    // totals: 200+300+1000 = 1500 cal; avg = 500
    assert!(stdout.contains("1500"));
    assert!(stdout.contains("500"));
    // exercise column appears because one day has exercise
    assert!(stdout.contains("Exercise"));
    assert!(stdout.contains("300"));
    // no maintenance_calories configured -> hint instead of deficit
    assert!(stdout.contains("maintenance_calories"));
    assert!(!stdout.contains("Deficit"));
}

#[test]
fn test_summary_deficit_with_config() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let config_dir = tempfile::TempDir::new().unwrap();
    let intake_config = config_dir.path().join("intake");
    std::fs::create_dir_all(&intake_config).unwrap();
    std::fs::write(
        intake_config.join("config.toml"),
        "maintenance_calories = 2400\n",
    )
    .unwrap();

    // food 1800, exercise 300 -> net 1500, tdee 2700, deficit 1200
    write_day_log(dir.path(), "2026-08-02", 1800, 50.0, 15.0, 300);

    let output = Command::new(binary())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .args(["--foods-dir", &fd_str, "--log-dir", &log_dir_str])
        .args(["summary", "2026-08-03", "--days", "7"])
        .output()
        .expect("failed to run intake");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(stdout.contains("Deficit"));
    assert!(stdout.contains("1200")); // per-day deficit, total, and avg
    assert!(!stdout.contains("maintenance_calories"));
}

#[test]
fn test_summary_no_entries() {
    let (stdout, success) = run_with_log_dir(&["summary", "2026-08-03", "--days", "7"]);
    assert!(success);
    assert!(stdout.contains("No entries in the last 7 days (ending 2026-08-03)"));
}

#[test]
fn test_summary_default_days_and_date() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let (stdout, success) = run(&["--foods-dir", &fd_str, "--log-dir", &log_dir_str, "summary"]);
    assert!(success, "summary failed: {}", stdout);
    assert!(stdout.contains("No entries in the last 7 days"));
}
