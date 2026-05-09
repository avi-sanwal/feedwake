use std::fs;
use std::process::Command;

use tempfile::TempDir;

fn feedwake_bin() -> &'static str {
    env!("CARGO_BIN_EXE_feedwake")
}

#[test]
fn version_flag_prints_package_version() {
    let output = Command::new(feedwake_bin())
        .arg("--version")
        .output()
        .expect("run feedwake --version");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "stdout should contain package version, got {stdout:?}"
    );
}

#[test]
fn verbose_scan_emits_progress_to_stderr() {
    let temp_dir = TempDir::new().expect("temp dir");
    let config_path = temp_dir.path().join("feedwake.toml");
    let state_db = temp_dir.path().join("feedwake.db");

    fs::write(
        &config_path,
        format!(
            r#"
feeds = []

[openclaw]
wake_url = "http://127.0.0.1:18789/hooks/feed-wake"
token_env = "OPENCLAW_HOOK_TOKEN"
mode = "now"

[scan]
timeout_seconds = 1
max_items_per_feed = 1
max_response_bytes = 1024
conditional_get = false
state_db = "{}"
"#,
            state_db.display()
        ),
    )
    .expect("write config");

    let output = Command::new(feedwake_bin())
        .args([
            "--verbose",
            "scan",
            "--config",
            config_path.to_str().expect("utf8 config path"),
        ])
        .output()
        .expect("run verbose scan");

    assert!(
        output.status.success(),
        "command failed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stdout.contains("scan complete: feeds=0"));
    assert!(
        stderr.contains("delivery queue empty"),
        "stderr should contain verbose progress, got {stderr:?}"
    );
}

#[test]
fn scan_log_file_writes_progress_without_cron_redirects() {
    let temp_dir = TempDir::new().expect("temp dir");
    let config_path = temp_dir.path().join("feedwake.toml");
    let state_db = temp_dir.path().join("feedwake.db");
    let log_file = temp_dir.path().join("logs/feedwake.log");

    fs::write(
        &config_path,
        format!(
            r#"
feeds = []

[openclaw]
wake_url = "http://127.0.0.1:18789/hooks/feed-wake"
token_env = "OPENCLAW_HOOK_TOKEN"
mode = "now"

[scan]
timeout_seconds = 1
max_items_per_feed = 1
max_response_bytes = 1024
conditional_get = false
state_db = "{}"
"#,
            state_db.display()
        ),
    )
    .expect("write config");

    let output = Command::new(feedwake_bin())
        .args([
            "--verbose",
            "scan",
            "--config",
            config_path.to_str().expect("utf8 config path"),
            "--log-file",
            log_file.to_str().expect("utf8 log path"),
        ])
        .output()
        .expect("run verbose scan with log file");

    assert!(
        output.status.success(),
        "command failed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let log = fs::read_to_string(&log_file).expect("read log file");
    assert!(log.contains("delivery queue empty"));
    assert!(log.contains("scan complete: feeds=0"));
}
