use std::path::{Path, PathBuf};
use std::process::Command;

fn foods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foods")
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_intake"))
}

fn run_in(args: &[&str], config_dir: &Path) -> (String, bool) {
    let output = Command::new(binary())
        .args(args)
        .env("XDG_CONFIG_HOME", config_dir)
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

fn run(args: &[&str]) -> (String, bool) {
    let config_dir = tempfile::TempDir::new().unwrap();
    run_in(args, config_dir.path())
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
    calories: &str,
    protein: &str,
    fiber: &str,
    exercise: u32,
) {
    let content = format!(
        "exercise_calories = {exercise}\n\n[[entries]]\nservings = 1.0\ncalories = {calories}\nprotein_g = {protein}\nfiber_g = {fiber}\nfat_g = 0.0\ncarbs_g = 0.0\nalcohol_g = 0.0\ntitle = \"Coffee\"\n"
    );
    std::fs::write(dir.join(format!("{date}.toml")), content).unwrap();
}

#[test]
fn test_list_all_foods() {
    let (stdout, success) = run_with_log_dir(&["list"]);
    assert!(success);
    assert!(stdout.contains("All Foods"));
    assert!(stdout.contains("Coffee"));
    assert!(stdout.contains("Oatmeal"));
    assert!(stdout.contains("Turkey Chili 98% Lean"));
}

#[test]
fn test_show_food() {
    let (stdout, success) = run_with_log_dir(&["show", "coffee"]);
    assert!(success);
    assert!(stdout.contains("Coffee (1 serving)"));
    assert!(stdout.contains("Cold Brew"));
    assert!(stdout.contains("Oat Milk"));
}

#[test]
fn test_show_food_notes() {
    let (stdout, success) = run_with_log_dir(&["show", "quest-bar"]);
    assert!(success);
    assert!(stdout.contains("Notes:"));
    assert!(stdout.contains("Store in a cool, dry place. Best eaten chilled."));
}

#[test]
fn test_show_food_no_notes() {
    let (stdout, success) = run_with_log_dir(&["show", "coffee"]);
    assert!(success);
    assert!(!stdout.contains("Notes:"));
}

#[test]
fn test_show_food_not_found() {
    let (_, success) = run_with_log_dir(&["show", "nonexistent-food"]);
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
fn test_adhoc_macros_optional_zero_defaults() {
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
        "Water",
        "1",
    ]);
    assert!(adhoc_ok, "adhoc failed: {}", adhoc_out);
    assert!(adhoc_out.contains("Water"));

    let today = chrono::Local::now().date_naive();
    let log_file = std::fs::read_to_string(dir.path().join(format!("{today}.toml")))
        .expect("log file written");
    assert!(log_file.contains("protein_g = 0"));
    assert!(log_file.contains("fiber_g = 0"));
    assert!(log_file.contains("fat_g = 0"));
    assert!(log_file.contains("carbs_g = 0"));
    assert!(log_file.contains("alcohol_g = 0"));

    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "log",
    ]);
    assert!(log_ok, "log failed: {}", log_out);
    assert!(log_out.contains("Water"));
    assert!(log_out.contains("250"));
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
    assert!(log_out.contains("Net"));
}

#[test]
fn test_log_net_row_with_exercise() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 300);

    let (stdout, success) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "log",
        "2026-08-02",
    ]);
    assert!(success, "log failed: {}", stdout);
    assert!(stdout.contains("Total"));
    assert!(stdout.contains("1800"));
    assert!(stdout.contains("-300"));
    assert!(stdout.contains("Net"));
    assert!(stdout.contains("1500"));
}

#[test]
fn test_log_fractional_exercise_rounds_display() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    std::fs::write(
        dir.path().join("2026-08-02.toml"),
        "exercise_calories = 300.5\n\n[[entries]]\nservings = 1.0\ncalories = 1800\nprotein_g = 50.0\nfiber_g = 15.0\nfat_g = 0.0\ncarbs_g = 0.0\nalcohol_g = 0.0\ntitle = \"Coffee\"\n",
    )
    .unwrap();

    let (stdout, success) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "log",
        "2026-08-02",
    ]);
    assert!(success, "log failed: {}", stdout);
    assert!(stdout.contains("1800"));
    // Exercise row rounds 300.5 away to 301; Net = 1800 - 300.5 = 1499.5 -> 1500
    assert!(stdout.contains("-301"));
    assert!(stdout.contains("1500"));
    assert!(!stdout.contains("300.5"));
}

#[test]
fn test_exercise_rows_hidden_when_calories_column_hidden() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let config_dir = tempfile::TempDir::new().unwrap();
    let intake_config = config_dir.path().join("intake");
    std::fs::create_dir_all(&intake_config).unwrap();
    std::fs::write(
        intake_config.join("config.toml"),
        "show_columns = [\"protein\", \"fat\"]\n",
    )
    .unwrap();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 300);

    let (stdout, success) = run_in(
        &[
            "--foods-dir",
            fd_str.as_str(),
            "--log-dir",
            log_dir_str.as_str(),
            "log",
            "2026-08-02",
        ],
        config_dir.path(),
    );
    assert!(success, "log failed: {}", stdout);
    assert!(stdout.contains("Total"));
    assert!(!stdout.contains("Exercise"));
    assert!(!stdout.contains("Net"));
    assert!(!stdout.contains("-300"));
}

#[test]
fn test_summary_exercise_column_hidden_when_calories_hidden() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let config_dir = tempfile::TempDir::new().unwrap();
    let intake_config = config_dir.path().join("intake");
    std::fs::create_dir_all(&intake_config).unwrap();
    std::fs::write(
        intake_config.join("config.toml"),
        "show_columns = [\"protein\", \"fat\"]\n",
    )
    .unwrap();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 300);

    let (stdout, success) = run_in(
        &[
            "--foods-dir",
            fd_str.as_str(),
            "--log-dir",
            log_dir_str.as_str(),
            "summary",
            "2026-08-03",
            "--days",
            "7",
        ],
        config_dir.path(),
    );
    assert!(success, "summary failed: {}", stdout);
    assert!(!stdout.contains("Exercise"));
    assert!(!stdout.contains("300"));
}

#[test]
fn test_summary_multi_day() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    // three logged days within a 7-day window; 08-02 also has exercise
    write_day_log(dir.path(), "2026-07-30", "200", "10.0", "4.0", 0);
    write_day_log(dir.path(), "2026-08-01", "300", "20.0", "8.0", 0);
    write_day_log(dir.path(), "2026-08-02", "1000", "50.0", "15.0", 300);

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
    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 300);

    let (stdout, success) = run_in(
        &[
            "--foods-dir",
            fd_str.as_str(),
            "--log-dir",
            log_dir_str.as_str(),
            "summary",
            "2026-08-03",
            "--days",
            "7",
        ],
        config_dir.path(),
    );
    assert!(success);
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

#[test]
fn test_adhoc_with_fat_carbs_alcohol() {
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
        "--fat",
        "9",
        "--carbs",
        "20",
        "--alcohol",
        "5",
        "Beer and nuts",
        "2",
    ]);
    assert!(adhoc_ok, "adhoc failed: {}", adhoc_out);
    assert!(adhoc_out.contains("Beer and nuts"));

    // all macros are stored per-serving, even alcohol
    let today = chrono::Local::now().date_naive();
    let log_file = std::fs::read_to_string(dir.path().join(format!("{today}.toml")))
        .expect("log file written");
    assert!(log_file.contains("fat_g = 9"));
    assert!(log_file.contains("carbs_g = 20"));
    assert!(log_file.contains("alcohol_g = 5"));

    // default view shows fat/carbs (scaled by 2 servings) but not alcohol
    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "log",
    ]);
    assert!(log_ok, "log failed: {}", log_out);
    assert!(!log_out.contains("Alcohol(g)"));

    // 2 servings: calories 500, fat 18.0, carbs 40.0, protein 24.0, fiber 6.0
    let row = log_out
        .lines()
        .find(|l| l.contains("Beer and nuts"))
        .expect("adhoc row present");
    let cells: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(cells[cells.len() - 6], "2", "servings cell in row: {row}");
    assert!(row.contains("500"), "calories in row: {row}");
    assert!(row.contains("18.0"), "fat in row: {row}");
    assert!(row.contains("40.0"), "carbs in row: {row}");
    assert!(row.contains("24.0"), "protein in row: {row}");
    assert!(row.contains("6.0"), "fiber in row: {row}");
}

#[test]
fn test_log_default_columns_include_fat_carbs_not_alcohol() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 0);

    let (stdout, success) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "log",
        "2026-08-02",
    ]);
    assert!(success, "log failed: {}", stdout);
    assert!(stdout.contains("Fat(g)"));
    assert!(stdout.contains("Carbs(g)"));
    assert!(!stdout.contains("Alcohol(g)"));
}

#[test]
fn test_show_columns_config_filters_columns() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let config_dir = tempfile::TempDir::new().unwrap();
    let intake_config = config_dir.path().join("intake");
    std::fs::create_dir_all(&intake_config).unwrap();
    std::fs::write(
        intake_config.join("config.toml"),
        "show_columns = [\"calories\", \"fat\", \"alcohol\"]\n",
    )
    .unwrap();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 0);

    let (stdout, success) = run_in(
        &[
            "--foods-dir",
            fd_str.as_str(),
            "--log-dir",
            log_dir_str.as_str(),
            "log",
            "2026-08-02",
        ],
        config_dir.path(),
    );
    assert!(success, "log failed: {}", stdout);
    assert!(stdout.contains("Calories"));
    assert!(stdout.contains("Fat(g)"));
    assert!(stdout.contains("Alcohol(g)"));
    assert!(!stdout.contains("Protein(g)"));
    assert!(!stdout.contains("Carbs(g)"));
}

#[test]
fn test_summary_respects_show_columns() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let config_dir = tempfile::TempDir::new().unwrap();
    let intake_config = config_dir.path().join("intake");
    std::fs::create_dir_all(&intake_config).unwrap();
    std::fs::write(
        intake_config.join("config.toml"),
        "show_columns = [\"fat\", \"alcohol\"]\n",
    )
    .unwrap();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 0);

    let (stdout, success) = run_in(
        &[
            "--foods-dir",
            fd_str.as_str(),
            "--log-dir",
            log_dir_str.as_str(),
            "summary",
            "2026-08-03",
            "--days",
            "7",
        ],
        config_dir.path(),
    );
    assert!(success, "summary failed: {}", stdout);
    assert!(stdout.contains("Fat(g)"));
    assert!(stdout.contains("Alcohol(g)"));
    assert!(!stdout.contains("Calories"));
    assert!(!stdout.contains("Protein(g)"));
    assert!(!stdout.contains("Carbs(g)"));
}

#[test]
fn test_min_fat_target_colors_total_row() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let config_dir = tempfile::TempDir::new().unwrap();
    let intake_config = config_dir.path().join("intake");
    std::fs::create_dir_all(&intake_config).unwrap();
    std::fs::write(intake_config.join("config.toml"), "min_fat = 50\n").unwrap();

    // day log with 30g fat total -> below min_fat -> yellow
    std::fs::write(
        dir.path().join("2026-08-02.toml"),
        "exercise_calories = 0\n\n[[entries]]\nservings = 1.0\ncalories = 500\nprotein_g = 10\nfiber_g = 2\nfat_g = 30\ncarbs_g = 20\nalcohol_g = 0\ntitle = \"Nuts\"\n",
    )
    .unwrap();

    let (stdout, success) = run_in(
        &[
            "--foods-dir",
            fd_str.as_str(),
            "--log-dir",
            log_dir_str.as_str(),
            "log",
            "2026-08-02",
        ],
        config_dir.path(),
    );
    assert!(success, "log failed: {}", stdout);
    assert!(stdout.contains("\u{1b}[33m")); // yellow: fat under min target
}

#[test]
fn test_max_fat_target_colors_total_row_red() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let config_dir = tempfile::TempDir::new().unwrap();
    let intake_config = config_dir.path().join("intake");
    std::fs::create_dir_all(&intake_config).unwrap();
    std::fs::write(intake_config.join("config.toml"), "max_fat = 50\n").unwrap();

    // day log with 60g fat total -> above max_fat -> red
    std::fs::write(
        dir.path().join("2026-08-02.toml"),
        "exercise_calories = 0\n\n[[entries]]\nservings = 1.0\ncalories = 500\nprotein_g = 10\nfiber_g = 2\nfat_g = 60\ncarbs_g = 20\nalcohol_g = 0\ntitle = \"Nuts\"\n",
    )
    .unwrap();

    let (stdout, success) = run_in(
        &[
            "--foods-dir",
            fd_str.as_str(),
            "--log-dir",
            log_dir_str.as_str(),
            "log",
            "2026-08-02",
        ],
        config_dir.path(),
    );
    assert!(success, "log failed: {}", stdout);
    assert!(stdout.contains("\u{1b}[31m")); // red: fat over max target
}

#[test]
fn test_show_columns_config_filters_list_and_show() {
    let fd_str = foods_dir().to_string_lossy().to_string();

    let config_dir = tempfile::TempDir::new().unwrap();
    let intake_config = config_dir.path().join("intake");
    std::fs::create_dir_all(&intake_config).unwrap();
    std::fs::write(
        intake_config.join("config.toml"),
        "show_columns = [\"calories\", \"alcohol\"]\n",
    )
    .unwrap();

    let (list_out, list_ok) = run_in(&["--foods-dir", fd_str.as_str(), "list"], config_dir.path());
    assert!(list_ok, "list failed: {}", list_out);
    assert!(list_out.contains("Cal/serv"));
    assert!(list_out.contains("Alcohol(g)"));
    assert!(!list_out.contains("Protein(g)"));
    assert!(!list_out.contains("Fat(g)"));
    assert!(!list_out.contains("Carbs(g)"));

    let (show_out, show_ok) = run_in(
        &["--foods-dir", fd_str.as_str(), "show", "coffee"],
        config_dir.path(),
    );
    assert!(show_ok, "show failed: {}", show_out);
    assert!(show_out.contains("Calories"));
    assert!(show_out.contains("Alcohol(g)"));
    assert!(!show_out.contains("Protein(g)"));
    assert!(!show_out.contains("Fat(g)"));
    assert!(!show_out.contains("Carbs(g)"));
}
