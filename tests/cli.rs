use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn foods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foods")
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_intake"))
}

fn run_in_env_full(
    args: &[&str],
    config_dir: &Path,
    envs: &[(&str, &str)],
) -> (String, String, bool) {
    let mut cmd = Command::new(binary());
    cmd.args(args)
        .env("XDG_CONFIG_HOME", config_dir)
        // Explicit closed stdin: `output()` already nulls it, but state the
        // contract so prompts EOF instead of ever reading a terminal.
        .stdin(Stdio::null());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let output = cmd.output().expect("failed to run intake");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();
    if !success {
        eprintln!("stderr: {}", stderr);
    }
    (stdout, stderr, success)
}

fn run_in_env(args: &[&str], config_dir: &Path, envs: &[(&str, &str)]) -> (String, bool) {
    let (stdout, _, success) = run_in_env_full(args, config_dir, envs);
    (stdout, success)
}

fn run_in_env_stdin(
    args: &[&str],
    config_dir: &Path,
    envs: &[(&str, &str)],
    stdin: &str,
) -> (String, bool) {
    let mut cmd = Command::new(binary());
    cmd.args(args)
        .env("XDG_CONFIG_HOME", config_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("failed to spawn intake");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())
        .expect("failed to write stdin");
    let output = child.wait_with_output().expect("failed to wait for intake");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();
    if !success {
        eprintln!("stderr: {}", stderr);
    }
    (stdout, success)
}

fn run_in(args: &[&str], config_dir: &Path) -> (String, bool) {
    // Tests pipe stdout, so force colors to keep ANSI behavior deterministic.
    run_in_env(args, config_dir, &[("CLICOLOR_FORCE", "1")])
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

/// The table row whose Item cell (column 1) equals `title` (single-word
/// titles only); panics if no such row exists.
fn item_row<'a>(stdout: &'a str, title: &str) -> &'a str {
    stdout
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(title))
        .unwrap_or_else(|| panic!("no row for {title:?} in:\n{stdout}"))
}

fn write_editor_script(config_dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = config_dir.join(name);
    std::fs::write(&path, body).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

/// An editor script that overwrites the temp file with `content`.
fn script_writes(content: &str) -> String {
    format!("#!/bin/sh\ncat > \"$1\" <<'EOF'\n{content}\nEOF\n")
}

/// An editor script that writes `first` on its first invocation and `second`
/// on later ones (via a marker file at `state_path`).
fn script_sequence(state_path: &Path, first: &str, second: &str) -> String {
    format!(
        "#!/bin/sh\nif [ -f \"{}\" ]; then\ncat > \"$1\" <<'EOF'\n{second}\nEOF\nelse\ntouch \"{}\"\ncat > \"$1\" <<'EOF'\n{first}\nEOF\nfi\n",
        state_path.display(),
        state_path.display()
    )
}

const VALID_FOOD: &str = "title = \"Test Food\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\nquantity = \"1 cup\"\nprotein_g = 10.0\nfiber_g = 5.0\ncalories = 200\nfat_g = 5.0\ncarbs_g = 30.0\nalcohol_g = 0.0\n";

const EDITED_FOOD: &str = "title = \"Coffee v2\"\nservings = 1\n\n[[ingredients]]\nname = \"Cold Brew\"\nquantity = \"100g\"\nprotein_g = 0\nfiber_g = 0\ncalories = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n\n[[ingredients]]\nname = \"Oat Milk\"\nquantity = \"50g\"\nprotein_g = 1\nfiber_g = 0\ncalories = 24\nfat_g = 0.6\ncarbs_g = 3.3\nalcohol_g = 0\n";

/// A fresh temp dir holding a copy of the named fixture food.
fn temp_foods_with_fixture(name: &str) -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::copy(
        foods_dir().join(format!("{name}.toml")),
        dir.path().join(format!("{name}.toml")),
    )
    .unwrap();
    dir
}

#[test]
fn test_list_all_foods() {
    let (stdout, success) = run_with_log_dir(&["food", "list"]);
    assert!(success);
    assert!(stdout.contains("All Foods"));
    assert!(stdout.contains("Coffee"));
    assert!(stdout.contains("Oatmeal"));
    assert!(stdout.contains("Turkey Chili 98% Lean"));
}

#[test]
fn test_show_food() {
    let (stdout, success) = run_with_log_dir(&["food", "show", "coffee"]);
    assert!(success);
    assert!(stdout.contains("Coffee (1 serving)"));
    assert!(stdout.contains("Cold Brew"));
    assert!(stdout.contains("Oat Milk"));
}

fn run_log_with_env(envs: &[(&str, &str)]) -> (String, bool) {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 0);

    let config_dir = tempfile::TempDir::new().unwrap();
    run_in_env(
        &[
            "--foods-dir",
            &fd_str,
            "--log-dir",
            &log_dir_str,
            "day",
            "2026-08-02",
        ],
        config_dir.path(),
        envs,
    )
}

#[test]
fn test_piped_output_has_no_ansi() {
    let (stdout, success) = run_log_with_env(&[]);
    assert!(success, "day failed: {}", stdout);
    assert!(!stdout.contains('\x1b'));
    assert!(stdout.contains("Total"));
}

#[test]
fn test_no_color_env_suppresses_ansi_even_with_force() {
    let (stdout, success) = run_log_with_env(&[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")]);
    assert!(success, "day failed: {}", stdout);
    assert!(!stdout.contains('\x1b'));
}

#[test]
fn test_force_color_adds_ansi_when_piped() {
    let (stdout, success) = run_log_with_env(&[("CLICOLOR_FORCE", "1")]);
    assert!(success, "day failed: {}", stdout);
    assert!(stdout.contains('\x1b'));
}

#[test]
fn test_show_food_notes() {
    let (stdout, success) = run_with_log_dir(&["food", "show", "quest-bar"]);
    assert!(success);
    assert!(stdout.contains("Notes:"));
    assert!(stdout.contains("Store in a cool, dry place. Best eaten chilled."));
}

#[test]
fn test_show_food_no_notes() {
    let (stdout, success) = run_with_log_dir(&["food", "show", "coffee"]);
    assert!(success);
    assert!(!stdout.contains("Notes:"));
}

#[test]
fn test_show_food_not_found() {
    let (_, success) = run_with_log_dir(&["food", "show", "nonexistent-food"]);
    assert!(!success);
}

#[test]
fn test_day_no_entries() {
    let (stdout, success) = run_with_log_dir(&["day", "2026-01-01"]);
    assert!(success);
    assert!(stdout.contains("No entries for 2026-01-01"));
}

#[test]
fn test_log_and_day_workflow() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();

    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "log",
        "coffee",
        "2",
    ]);
    assert!(log_ok, "log failed: {}", log_out);
    assert!(log_out.contains("Coffee"));

    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "day",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(day_out.contains("Coffee"));
    assert!(day_out.contains("48")); // 2 servings × 24 cal
}

#[test]
fn test_bare_intake_shows_today() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();

    let today = chrono::Local::now().date_naive();
    write_day_log(
        dir.path(),
        &today.format("%Y-%m-%d").to_string(),
        "1800",
        "50.0",
        "15.0",
        0,
    );

    let (stdout, success) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
    ]);
    assert!(success, "bare intake failed: {}", stdout);
    assert!(stdout.contains(&today.format("%Y-%m-%d").to_string()));
    assert!(stdout.contains("Coffee"));
    assert!(stdout.contains("Total"));
}

#[test]
fn test_log_date_flag_targets_day() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();

    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "log",
        "coffee",
        "--date",
        "2026-08-01",
    ]);
    assert!(log_ok, "log failed: {}", log_out);

    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-01",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(day_out.contains("Coffee"));
    assert!(day_out.contains("24"));
}

#[test]
fn test_log_invalid_date_errors() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let (_, success) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "log",
        "coffee",
        "--date",
        "yesterday",
    ]);
    assert!(!success);
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
        "log",
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

    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "day",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(day_out.contains("Greek yogurt"));
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
        "log",
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

    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "day",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(day_out.contains("Water"));
    assert!(day_out.contains("250"));
}

#[test]
fn test_log_macros_win_over_food_name() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();

    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "log",
        "coffee",
        "--calories",
        "500",
    ]);
    assert!(log_ok, "log failed: {}", log_out);

    let today = chrono::Local::now().date_naive();
    let log_file = std::fs::read_to_string(dir.path().join(format!("{today}.toml")))
        .expect("log file written");
    // adhoc path: the name is the title and macros are exactly as given
    assert!(log_file.contains("title = \"coffee\""));
    assert!(log_file.contains("calories = 500"));
    assert!(log_file.contains("protein_g = 0"));
}

#[test]
fn test_log_unknown_name_without_macros_errors() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let config_dir = tempfile::TempDir::new().unwrap();

    let (_, stderr, success) = run_in_env_full(
        &[
            "--foods-dir",
            &foods_dir().to_string_lossy(),
            "--log-dir",
            &log_dir_str,
            "log",
            "nonexistent-food",
        ],
        config_dir.path(),
        &[],
    );
    assert!(!success);
    assert!(stderr.contains("no food 'nonexistent-food' found"));
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

    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "day",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(day_out.contains("300"));
    assert!(day_out.contains("Exercise"));
    assert!(day_out.contains("Net"));
}

#[test]
fn test_day_net_row_with_exercise() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 300);

    let (stdout, success) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-02",
    ]);
    assert!(success, "day failed: {}", stdout);
    assert!(stdout.contains("Total"));
    assert!(stdout.contains("1800"));
    assert!(stdout.contains("-300"));
    assert!(stdout.contains("Net"));
    assert!(stdout.contains("1500"));
}

#[test]
fn test_day_fractional_exercise_rounds_display() {
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
        "day",
        "2026-08-02",
    ]);
    assert!(success, "day failed: {}", stdout);
    assert!(stdout.contains("1800"));
    // Exercise row rounds 300.5 away to 301; Net = 1800 - 300.5 = 1499.5 -> 1500
    assert!(stdout.contains("-301"));
    assert!(stdout.contains("1500"));
    assert!(!stdout.contains("300.5"));
}

#[test]
fn test_day_unknown_field_errors() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    std::fs::write(
        dir.path().join("2026-08-02.toml"),
        "exercise_calories = 0\nbogus = 1\n",
    )
    .unwrap();

    let (_, success) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-02",
    ]);
    assert!(!success);
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
            "day",
            "2026-08-02",
        ],
        config_dir.path(),
    );
    assert!(success, "day failed: {}", stdout);
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

    // food 1800, exercise 300 -> net 1500, deficit = 2400 - 1500 = 900
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
    assert!(stdout.contains("900")); // per-day deficit, total, and avg
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
        "log",
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
    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &foods_dir().to_string_lossy(),
        "--log-dir",
        &log_dir_str,
        "day",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(!day_out.contains("Alcohol(g)"));

    // 2 servings: calories 500, fat 18.0, carbs 40.0, protein 24.0, fiber 6.0
    let row = day_out
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
fn test_day_total_row_does_not_sum_servings() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    // 2 servings of Chili (100 cal) + 1 serving of Oatmeal (50 cal)
    std::fs::write(
        dir.path().join("2026-08-02.toml"),
        "exercise_calories = 0\n\n[[entries]]\nservings = 2.0\ncalories = 100.0\nprotein_g = 10.0\nfiber_g = 4.0\nfat_g = 0.0\ncarbs_g = 0.0\nalcohol_g = 0.0\ntitle = \"Chili\"\n\n[[entries]]\nservings = 1.0\ncalories = 50.0\nprotein_g = 5.0\nfiber_g = 2.0\nfat_g = 0.0\ncarbs_g = 0.0\nalcohol_g = 0.0\ntitle = \"Oatmeal\"\n",
    )
    .unwrap();

    let (stdout, success) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-02",
    ]);
    assert!(success, "day failed: {}", stdout);

    // per-row servings cells remain (cell 0 is the entry number)
    let chili = item_row(&stdout, "Chili");
    assert_eq!(chili.split_whitespace().nth(2), Some("2"), "row: {chili}");
    let oatmeal = item_row(&stdout, "Oatmeal");
    assert_eq!(
        oatmeal.split_whitespace().nth(2),
        Some("1"),
        "row: {oatmeal}"
    );

    // total row: servings slot is blank, so the first macro value (calories,
    // 2*100 + 1*50 = 250) follows "Total" directly — not the summed "3"
    let total = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("Total"))
        .expect("total row present");
    let cells: Vec<&str> = total.split_whitespace().collect();
    assert_eq!(cells[0], "Total", "row: {total}");
    assert_eq!(
        cells[1], "250",
        "servings sum leaked into total row: {total}"
    );
}

#[test]
fn test_day_default_columns_include_fat_carbs_not_alcohol() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 0);

    let (stdout, success) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-02",
    ]);
    assert!(success, "day failed: {}", stdout);
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
            "day",
            "2026-08-02",
        ],
        config_dir.path(),
    );
    assert!(success, "day failed: {}", stdout);
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
            "day",
            "2026-08-02",
        ],
        config_dir.path(),
    );
    assert!(success, "day failed: {}", stdout);
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
            "day",
            "2026-08-02",
        ],
        config_dir.path(),
    );
    assert!(success, "day failed: {}", stdout);
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

    let (list_out, list_ok) = run_in(
        &["--foods-dir", fd_str.as_str(), "food", "list"],
        config_dir.path(),
    );
    assert!(list_ok, "list failed: {}", list_out);
    assert!(list_out.contains("Cal/serv"));
    assert!(list_out.contains("Alcohol(g)"));
    assert!(!list_out.contains("Protein(g)"));
    assert!(!list_out.contains("Fat(g)"));
    assert!(!list_out.contains("Carbs(g)"));

    let (show_out, show_ok) = run_in(
        &["--foods-dir", fd_str.as_str(), "food", "show", "coffee"],
        config_dir.path(),
    );
    assert!(show_ok, "show failed: {}", show_out);
    assert!(show_out.contains("Calories"));
    assert!(show_out.contains("Alcohol(g)"));
    assert!(!show_out.contains("Protein(g)"));
    assert!(!show_out.contains("Fat(g)"));
    assert!(!show_out.contains("Carbs(g)"));
}

#[test]
fn test_food_new_writes_file_with_editor() {
    let foods_dir_tmp = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();
    let editor = write_editor_script(config_dir.path(), "editor.sh", &script_writes(VALID_FOOD));

    let (stdout, success) = run_in_env_stdin(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "new",
            "my-food",
        ],
        config_dir.path(),
        &[("EDITOR", editor.to_str().unwrap())],
        "y\n",
    );
    assert!(success, "food new failed: {}", stdout);
    assert!(stdout.contains("Wrote"));

    let written = std::fs::read_to_string(foods_dir_tmp.path().join("my-food.toml")).unwrap();
    assert!(written.contains("title = \"Test Food\""));
    assert!(written.contains("calories = 200"));
}

#[test]
fn test_food_new_yes_flag_skips_confirmation() {
    let foods_dir_tmp = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();
    let editor = write_editor_script(config_dir.path(), "editor.sh", &script_writes(VALID_FOOD));

    let (stdout, success) = run_in_env(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "new",
            "my-food",
            "--yes",
        ],
        config_dir.path(),
        &[("EDITOR", editor.to_str().unwrap())],
    );
    assert!(success, "food new failed: {}", stdout);
    assert!(stdout.contains("Wrote"));
    assert!(foods_dir_tmp.path().join("my-food.toml").exists());
}

#[test]
fn test_food_new_collision_errors() {
    let config_dir = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();

    let (_, stderr, success) = run_in_env_full(
        &[
            "--foods-dir",
            &foods_dir().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "new",
            "coffee",
        ],
        config_dir.path(),
        &[],
    );
    assert!(!success);
    assert!(stderr.contains("already exists"));
    assert!(stderr.contains("food edit coffee"));
}

#[test]
fn test_food_new_unchanged_aborts() {
    let foods_dir_tmp = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();
    let editor = write_editor_script(config_dir.path(), "editor.sh", "#!/bin/sh\nexit 0\n");

    let (stdout, success) = run_in_env(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "new",
            "my-food",
        ],
        config_dir.path(),
        &[("EDITOR", editor.to_str().unwrap())],
    );
    assert!(success, "unchanged abort should exit 0: {}", stdout);
    assert!(stdout.contains("Nothing written"));
    assert!(!foods_dir_tmp.path().join("my-food.toml").exists());
}

#[test]
fn test_food_new_editor_failure_errors() {
    let foods_dir_tmp = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();

    let (_, success) = run_in_env(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "new",
            "my-food",
        ],
        config_dir.path(),
        &[("EDITOR", "/nonexistent/editor")],
    );
    assert!(!success);
    assert!(!foods_dir_tmp.path().join("my-food.toml").exists());
}

#[test]
fn test_food_new_invalid_then_valid_retries() {
    let foods_dir_tmp = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();
    let state = config_dir.path().join("state");
    let script = script_sequence(&state, "this is not valid toml\n", VALID_FOOD);
    let editor = write_editor_script(config_dir.path(), "editor.sh", &script);

    let (stdout, success) = run_in_env_stdin(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "new",
            "my-food",
        ],
        config_dir.path(),
        &[("EDITOR", editor.to_str().unwrap())],
        "y\n",
    );
    assert!(success, "food new failed: {}", stdout);
    assert!(stdout.contains("Wrote"));
    assert!(foods_dir_tmp.path().join("my-food.toml").exists());
}

#[test]
fn test_food_new_reject_writes_nothing() {
    let foods_dir_tmp = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();
    let editor = write_editor_script(config_dir.path(), "editor.sh", &script_writes(VALID_FOOD));

    let (stdout, success) = run_in_env_stdin(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "new",
            "my-food",
        ],
        config_dir.path(),
        &[("EDITOR", editor.to_str().unwrap())],
        "n\n",
    );
    assert!(success, "reject should exit 0: {}", stdout);
    assert!(stdout.contains("Nothing written"));
    assert!(!foods_dir_tmp.path().join("my-food.toml").exists());
}

#[test]
fn test_food_new_closed_stdin_cancels() {
    let foods_dir_tmp = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();
    let editor = write_editor_script(config_dir.path(), "editor.sh", &script_writes(VALID_FOOD));

    let (stdout, success) = run_in_env(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "new",
            "my-food",
        ],
        config_dir.path(),
        &[("EDITOR", editor.to_str().unwrap())],
    );
    assert!(success, "cancelled should exit 0: {}", stdout);
    assert!(stdout.contains("Nothing written"));
    assert!(!foods_dir_tmp.path().join("my-food.toml").exists());
}

#[test]
fn test_food_edit_roundtrip() {
    let foods_dir_tmp = temp_foods_with_fixture("coffee");
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();
    let editor = write_editor_script(config_dir.path(), "editor.sh", &script_writes(EDITED_FOOD));

    let (stdout, success) = run_in_env_stdin(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "edit",
            "coffee",
        ],
        config_dir.path(),
        &[("EDITOR", editor.to_str().unwrap())],
        "y\n",
    );
    assert!(success, "food edit failed: {}", stdout);
    assert!(stdout.contains("Wrote"));

    let written = std::fs::read_to_string(foods_dir_tmp.path().join("coffee.toml")).unwrap();
    assert!(written.contains("title = \"Coffee v2\""));
}

#[test]
fn test_food_edit_reject_keeps_file() {
    let foods_dir_tmp = temp_foods_with_fixture("coffee");
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();
    let editor = write_editor_script(config_dir.path(), "editor.sh", &script_writes(EDITED_FOOD));

    let (stdout, success) = run_in_env_stdin(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "edit",
            "coffee",
        ],
        config_dir.path(),
        &[("EDITOR", editor.to_str().unwrap())],
        "n\n",
    );
    assert!(success, "reject should exit 0: {}", stdout);
    assert!(stdout.contains("Nothing written"));

    let written = std::fs::read_to_string(foods_dir_tmp.path().join("coffee.toml")).unwrap();
    assert!(written.contains("title = \"Coffee\""));
    assert!(!written.contains("Coffee v2"));
}

#[test]
fn test_food_edit_not_found() {
    let foods_dir_tmp = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();

    let (_, stderr, success) = run_in_env_full(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "edit",
            "nonexistent-food",
        ],
        config_dir.path(),
        &[("EDITOR", "/bin/true")],
    );
    assert!(!success);
    assert!(stderr.contains("not found"));
}

#[test]
fn test_food_rm_removes_file_with_confirmation() {
    let foods_dir_tmp = temp_foods_with_fixture("coffee");
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();

    let (stdout, success) = run_in_env_stdin(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "rm",
            "coffee",
        ],
        config_dir.path(),
        &[],
        "y\n",
    );
    assert!(success, "food rm failed: {}", stdout);
    assert!(stdout.contains("Removed"));
    assert!(!foods_dir_tmp.path().join("coffee.toml").exists());
}

#[test]
fn test_food_rm_yes_flag_skips_confirmation() {
    let foods_dir_tmp = temp_foods_with_fixture("coffee");
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();

    let (stdout, success) = run_in_env(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "rm",
            "coffee",
            "--yes",
        ],
        config_dir.path(),
        &[],
    );
    assert!(success, "food rm failed: {}", stdout);
    assert!(stdout.contains("Removed"));
    assert!(!foods_dir_tmp.path().join("coffee.toml").exists());
}

#[test]
fn test_food_rm_reject_keeps_file() {
    let foods_dir_tmp = temp_foods_with_fixture("coffee");
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();

    let (stdout, success) = run_in_env_stdin(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "rm",
            "coffee",
        ],
        config_dir.path(),
        &[],
        "n\n",
    );
    assert!(success, "reject should exit 0: {}", stdout);
    assert!(stdout.contains("Nothing removed"));
    assert!(foods_dir_tmp.path().join("coffee.toml").exists());
}

#[test]
fn test_food_rm_closed_stdin_cancels() {
    let foods_dir_tmp = temp_foods_with_fixture("coffee");
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();

    let (stdout, success) = run_in_env(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "rm",
            "coffee",
        ],
        config_dir.path(),
        &[],
    );
    assert!(success, "cancelled should exit 0: {}", stdout);
    assert!(stdout.contains("Nothing removed"));
    assert!(foods_dir_tmp.path().join("coffee.toml").exists());
}

#[test]
fn test_food_rm_not_found() {
    let foods_dir_tmp = tempfile::TempDir::new().unwrap();
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();

    let (_, stderr, success) = run_in_env_full(
        &[
            "--foods-dir",
            &foods_dir_tmp.path().to_string_lossy(),
            "--log-dir",
            &log_dir.path().to_string_lossy(),
            "food",
            "rm",
            "nonexistent-food",
            "--yes",
        ],
        config_dir.path(),
        &[],
    );
    assert!(!success);
    assert!(stderr.contains("not found"));
}

#[test]
fn test_food_rm_leaves_log_entries_intact() {
    let foods_dir_tmp = temp_foods_with_fixture("coffee");
    let log_dir = tempfile::TempDir::new().unwrap();
    let config_dir = tempfile::TempDir::new().unwrap();

    let fd = foods_dir_tmp.path().to_string_lossy().to_string();
    let ld = log_dir.path().to_string_lossy().to_string();

    let (log_out, log_ok) = run_in_env(
        &["--foods-dir", &fd, "--log-dir", &ld, "log", "coffee"],
        config_dir.path(),
        &[],
    );
    assert!(log_ok, "log failed: {}", log_out);

    let (rm_out, rm_ok) = run_in_env(
        &[
            "--foods-dir",
            &fd,
            "--log-dir",
            &ld,
            "food",
            "rm",
            "coffee",
            "--yes",
        ],
        config_dir.path(),
        &[],
    );
    assert!(rm_ok, "food rm failed: {}", rm_out);
    assert!(!foods_dir_tmp.path().join("coffee.toml").exists());

    let (day_out, day_ok) = run_in_env(
        &["--foods-dir", &fd, "--log-dir", &ld, "day"],
        config_dir.path(),
        &[],
    );
    assert!(day_ok, "day failed: {}", day_out);
    assert!(day_out.contains("Coffee"));
    assert!(day_out.contains("24")); // 1 serving × 24 cal, from the removed food
}

#[test]
fn test_day_shows_entry_numbers() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    std::fs::write(
        dir.path().join("2026-08-02.toml"),
        "exercise_calories = 0\n\n[[entries]]\nservings = 1.0\ncalories = 100\nprotein_g = 10.0\nfiber_g = 4.0\nfat_g = 0.0\ncarbs_g = 0.0\nalcohol_g = 0.0\ntitle = \"Chili\"\n\n[[entries]]\nservings = 1.0\ncalories = 50\nprotein_g = 5.0\nfiber_g = 2.0\nfat_g = 0.0\ncarbs_g = 0.0\nalcohol_g = 0.0\ntitle = \"Oatmeal\"\n",
    )
    .unwrap();

    let (stdout, success) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-02",
    ]);
    assert!(success, "day failed: {}", stdout);
    assert!(
        stdout
            .lines()
            .any(|l| l.contains('#') && l.contains("Item")),
        "header row missing: {stdout}"
    );

    let chili = item_row(&stdout, "Chili");
    assert_eq!(chili.split_whitespace().nth(0), Some("1"), "row: {chili}");
    let oatmeal = item_row(&stdout, "Oatmeal");
    assert_eq!(
        oatmeal.split_whitespace().nth(0),
        Some("2"),
        "row: {oatmeal}"
    );
}

#[test]
fn test_rm_removes_entry_with_confirmation() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();
    let config_dir = tempfile::TempDir::new().unwrap();

    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "log",
        "coffee",
        "2",
        "--date",
        "2026-08-02",
    ]);
    assert!(log_ok, "log failed: {}", log_out);
    let (log_out, log_ok) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "log",
        "oatmeal",
        "1",
        "--date",
        "2026-08-02",
    ]);
    assert!(log_ok, "log failed: {}", log_out);

    let (rm_out, rm_ok) = run_in_env_stdin(
        &[
            "--foods-dir",
            &fd_str,
            "--log-dir",
            &log_dir_str,
            "rm",
            "2",
            "--date",
            "2026-08-02",
        ],
        config_dir.path(),
        &[],
        "y\n",
    );
    assert!(rm_ok, "rm failed: {}", rm_out);
    assert!(rm_out.contains("Removed entry 2 (Oatmeal, 1 serving, 418 kcal) from 2026-08-02"));
    let oatmeal_rows = rm_out
        .lines()
        .filter(|l| l.split_whitespace().nth(1) == Some("Oatmeal"))
        .count();
    assert_eq!(
        oatmeal_rows, 0,
        "removed entry must not appear in the day table: {rm_out}"
    );
    assert!(rm_out.contains("Coffee"));
}

#[test]
fn test_rm_yes_flag_skips_confirmation() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 0);
    std::fs::write(
        dir.path().join("2026-08-02.toml"),
        "exercise_calories = 0\n\n[[entries]]\nservings = 1.0\ncalories = 1800\nprotein_g = 50.0\nfiber_g = 15.0\nfat_g = 0.0\ncarbs_g = 0.0\nalcohol_g = 0.0\ntitle = \"Coffee\"\n\n[[entries]]\nservings = 1.0\ncalories = 100\nprotein_g = 10.0\nfiber_g = 4.0\nfat_g = 0.0\ncarbs_g = 0.0\nalcohol_g = 0.0\ntitle = \"Chili\"\n",
    )
    .unwrap();

    let (rm_out, rm_ok) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "rm",
        "1",
        "--date",
        "2026-08-02",
        "--yes",
    ]);
    assert!(rm_ok, "rm failed: {}", rm_out);
    assert!(rm_out.contains("Removed entry 1"));

    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-02",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(!day_out.contains("Coffee"));
    assert!(day_out.contains("Chili"));
}

#[test]
fn test_rm_reject_keeps_entry() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();
    let config_dir = tempfile::TempDir::new().unwrap();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 0);

    let (rm_out, rm_ok) = run_in_env_stdin(
        &[
            "--foods-dir",
            &fd_str,
            "--log-dir",
            &log_dir_str,
            "rm",
            "1",
            "--date",
            "2026-08-02",
        ],
        config_dir.path(),
        &[],
        "n\n",
    );
    assert!(rm_ok, "reject should exit 0: {}", rm_out);
    assert!(rm_out.contains("Nothing removed"));
    assert!(!rm_out.contains("Removed entry"));

    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-02",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(day_out.contains("Coffee"));
}

#[test]
fn test_rm_out_of_range_errors() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 0);

    let (_, stderr, success) = run_in_env_full(
        &[
            "--foods-dir",
            &fd_str,
            "--log-dir",
            &log_dir_str,
            "rm",
            "5",
            "--date",
            "2026-08-02",
            "--yes",
        ],
        tempfile::TempDir::new().unwrap().path(),
        &[],
    );
    assert!(!success);
    assert!(stderr.contains("entry 5 not found"));
    assert!(stderr.contains("1 entry"));
}

#[test]
fn test_rm_no_entries_errors() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    let (_, stderr, success) = run_in_env_full(
        &[
            "--foods-dir",
            &fd_str,
            "--log-dir",
            &log_dir_str,
            "rm",
            "1",
            "--date",
            "2026-08-02",
            "--yes",
        ],
        tempfile::TempDir::new().unwrap().path(),
        &[],
    );
    assert!(!success);
    assert!(stderr.contains("no entries for 2026-08-02"));
}

#[test]
fn test_rm_last_entry_removes_day_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 0);

    let (rm_out, rm_ok) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "rm",
        "1",
        "--date",
        "2026-08-02",
        "--yes",
    ]);
    assert!(rm_ok, "rm failed: {}", rm_out);
    assert!(rm_out.contains("Removed entry 1"));

    assert!(!dir.path().join("2026-08-02.toml").exists());

    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-02",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(day_out.contains("No entries for 2026-08-02"));
}

#[test]
fn test_rm_preserves_exercise() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_dir_str = dir.path().to_string_lossy().to_string();
    let fd_str = foods_dir().to_string_lossy().to_string();

    write_day_log(dir.path(), "2026-08-02", "1800", "50.0", "15.0", 300);

    let (rm_out, rm_ok) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "rm",
        "1",
        "--date",
        "2026-08-02",
        "--yes",
    ]);
    assert!(rm_ok, "rm failed: {}", rm_out);

    assert!(dir.path().join("2026-08-02.toml").exists());

    let (day_out, day_ok) = run(&[
        "--foods-dir",
        &fd_str,
        "--log-dir",
        &log_dir_str,
        "day",
        "2026-08-02",
    ]);
    assert!(day_ok, "day failed: {}", day_out);
    assert!(!day_out.contains("Coffee"));
    assert!(day_out.contains("Exercise"));
    assert!(day_out.contains("300"));
}
