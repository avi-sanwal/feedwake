use std::fs;
use std::process::Command;
use std::thread;

use tempfile::TempDir;
use tiny_http::{Header, Response, Server};

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

#[test]
fn verbose_dry_run_logs_item_decisions() {
    let temp_dir = TempDir::new().expect("temp dir");
    let config_path = temp_dir.path().join("feedwake.toml");
    let state_db = temp_dir.path().join("feedwake.db");

    let feed_server = Server::http("127.0.0.1:0").expect("feed server");
    let feed_url = format!("http://{}/feed.xml", feed_server.server_addr());
    thread::spawn(move || {
        let request = feed_server.recv().expect("feed request");
        request
            .respond(Response::from_string(decision_rss()).with_header(
                Header::from_bytes("Content-Type", "application/rss+xml").expect("content type"),
            ))
            .expect("feed response");
    });

    fs::write(
        &config_path,
        format!(
            r#"
[openclaw]
wake_url = "http://127.0.0.1:18789/hooks/feed-wake"
token_env = "OPENCLAW_HOOK_TOKEN"
mode = "now"

[scan]
timeout_seconds = 2
max_items_per_feed = 10
max_response_bytes = 65536
conditional_get = false
state_db = "{state_db}"

[[watchlist]]
symbol = "RELIANCE"
name = "Reliance Industries Limited"
aliases = ["RIL"]

[media_filters]
require_watchlist_match = true
keyword_groups = ["results"]

[[feeds]]
name = "Market Feed"
url = "{feed_url}"
source_type = "media"
filter_profile = "media_high_precision"
"#,
            state_db = state_db.display()
        ),
    )
    .expect("write config");

    let output = Command::new(feedwake_bin())
        .args([
            "--verbose",
            "scan",
            "--config",
            config_path.to_str().expect("utf8 config path"),
            "--dry-run",
        ])
        .output()
        .expect("run verbose dry-run scan");

    assert!(
        output.status.success(),
        "command failed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stdout.contains("dry-run match: Feed alert: Market Feed published"));
    assert!(stderr.contains("feed item found: feed=\"Market Feed\" title=\"Reliance results\""));
    assert!(stderr.contains(
        "feed item matched: feed=\"Market Feed\" title=\"Reliance results\" reason=media_high_precision entity=RELIANCE action=dry_run"
    ));
    assert!(stderr.contains(
        "feed item discarded: feed=\"Market Feed\" title=\"Generic market update\" reason=watchlist_miss"
    ));
}

fn decision_rss() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Market Feed</title>
    <item>
      <title>Reliance results</title>
      <link>https://example.com/reliance-results</link>
      <description>RIL results update</description>
    </item>
    <item>
      <title>Generic market update</title>
      <link>https://example.com/generic-update</link>
      <description>Market commentary</description>
    </item>
  </channel>
</rss>"#
}
