use feedwake::app::{run_scan_with_options, ScanOptions};
use serde_json::Value;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Header, Response, Server};

#[test]
fn scan_delivers_limited_article_batch_and_leaves_excess_for_next_run() {
    let temp_dir = TempDir::new().expect("temp dir");
    let state_db = temp_dir.path().join("feedwake.db");

    let feed_server = Server::http("127.0.0.1:0").expect("feed server");
    let feed_url = format!("http://{}/feed.xml", feed_server.server_addr());
    thread::spawn(move || {
        for _ in 0..2 {
            let request = feed_server.recv().expect("feed request");
            request
                .respond(
                    Response::from_string(rss_with_four_items()).with_header(
                        Header::from_bytes("Content-Type", "application/rss+xml")
                            .expect("content type"),
                    ),
                )
                .expect("feed response");
        }
    });

    let hook_server = Server::http("127.0.0.1:0").expect("hook server");
    let hook_url = format!("http://{}/hooks/feed-wake", hook_server.server_addr());
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for _ in 0..2 {
            let mut request = hook_server.recv().expect("hook request");
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).expect("body");
            request
                .respond(Response::from_string("ok"))
                .expect("hook response");
            sender.send(body).expect("send body");
        }
    });

    let config_path = temp_dir.path().join("feedwake.toml");
    fs::write(
        &config_path,
        format!(
            r#"
[openclaw]
wake_url = "{hook_url}"
token_env = "OPENCLAW_HOOK_TOKEN"
mode = "now"
max_articles_per_wake = 3

[scan]
timeout_seconds = 2
max_items_per_feed = 10
max_response_bytes = 65536
conditional_get = false
state_db = "{state_db}"

[[feeds]]
name = "SEBI"
url = "{feed_url}"
source_type = "sebi"
filter_profile = "authority_passthrough"
"#,
            state_db = state_db.display()
        ),
    )
    .expect("write config");

    std::env::set_var("OPENCLAW_HOOK_TOKEN", "secret-token");

    let first = run_scan_with_options(
        Some(&config_path),
        ScanOptions {
            dry_run: false,
            verbose: false,
        },
    )
    .expect("first scan");
    let first_body = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("first hook body");

    let second = run_scan_with_options(
        Some(&config_path),
        ScanOptions {
            dry_run: false,
            verbose: false,
        },
    )
    .expect("second scan");
    let second_body = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("second hook body");

    let first_payload: Value = serde_json::from_str(&first_body).expect("first json");
    let second_payload: Value = serde_json::from_str(&second_body).expect("second json");

    assert_eq!(first.events_enqueued, 4);
    assert_eq!(first.events_delivered, 3);
    assert_eq!(first_payload["articleCount"], 3);
    assert!(first_payload["sessionKey"]
        .as_str()
        .expect("session key")
        .starts_with("hook:feedwake:"));
    assert_eq!(first_payload["articles"][0]["description"], "Summary 1");
    assert_eq!(second.events_enqueued, 0);
    assert_eq!(second.events_delivered, 1);
    assert_eq!(second_payload["articleCount"], 1);
    assert!(second_payload["sessionKey"]
        .as_str()
        .expect("session key")
        .starts_with("hook:feedwake:"));
    assert_ne!(first_payload["sessionKey"], second_payload["sessionKey"]);
    assert_eq!(second_payload["articles"][0]["title"], "Alert 4");
}

#[test]
fn dry_run_with_configured_state_db_does_not_mutate_persistent_state() {
    let temp_dir = TempDir::new().expect("temp dir");
    let state_db = temp_dir.path().join("feedwake.db");

    let feed_server = Server::http("127.0.0.1:0").expect("feed server");
    let feed_url = format!("http://{}/feed.xml", feed_server.server_addr());
    thread::spawn(move || {
        for _ in 0..2 {
            let request = feed_server.recv().expect("feed request");
            request
                .respond(
                    Response::from_string(rss_with_two_items()).with_header(
                        Header::from_bytes("Content-Type", "application/rss+xml")
                            .expect("content type"),
                    ),
                )
                .expect("feed response");
        }
    });

    let hook_server = Server::http("127.0.0.1:0").expect("hook server");
    let hook_url = format!("http://{}/hooks/feed-wake", hook_server.server_addr());
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut request = hook_server.recv().expect("hook request");
        let mut body = String::new();
        request.as_reader().read_to_string(&mut body).expect("body");
        request
            .respond(Response::from_string("ok"))
            .expect("hook response");
        sender.send(body).expect("send body");
    });

    let config_path = temp_dir.path().join("feedwake.toml");
    fs::write(
        &config_path,
        format!(
            r#"
[openclaw]
wake_url = "{hook_url}"
token_env = "OPENCLAW_HOOK_TOKEN"
mode = "now"
max_articles_per_wake = 10

[scan]
timeout_seconds = 2
max_items_per_feed = 10
max_response_bytes = 65536
conditional_get = false
state_db = "{state_db}"

[[feeds]]
name = "SEBI"
url = "{feed_url}"
source_type = "sebi"
filter_profile = "authority_passthrough"
"#,
            state_db = state_db.display()
        ),
    )
    .expect("write config");

    std::env::set_var("OPENCLAW_HOOK_TOKEN", "secret-token");

    let dry_run = run_scan_with_options(
        Some(&config_path),
        ScanOptions {
            dry_run: true,
            verbose: false,
        },
    )
    .expect("dry-run scan");

    assert_eq!(dry_run.events_enqueued, 2);
    assert_eq!(dry_run.events_delivered, 0);
    assert!(
        !state_db.exists(),
        "dry-run should not create or mutate configured state db"
    );

    let normal = run_scan_with_options(
        Some(&config_path),
        ScanOptions {
            dry_run: false,
            verbose: false,
        },
    )
    .expect("normal scan");
    let body = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("hook body");
    let payload: Value = serde_json::from_str(&body).expect("json");

    assert_eq!(normal.events_enqueued, 2);
    assert_eq!(normal.events_delivered, 2);
    assert_eq!(payload["articleCount"], 2);
    assert_eq!(payload["articles"][0]["title"], "Alert 1");
}

fn rss_with_four_items() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>SEBI</title>
    <link>https://example.com</link>
    <description>SEBI feed</description>
    <item>
      <title>Alert 1</title>
      <link>https://example.com/a1</link>
      <description>Summary 1</description>
      <pubDate>Sat, 09 May 2026 10:00:00 GMT</pubDate>
    </item>
    <item>
      <title>Alert 2</title>
      <link>https://example.com/a2</link>
      <description>Summary 2</description>
      <pubDate>Sat, 09 May 2026 10:01:00 GMT</pubDate>
    </item>
    <item>
      <title>Alert 3</title>
      <link>https://example.com/a3</link>
      <description>Summary 3</description>
      <pubDate>Sat, 09 May 2026 10:02:00 GMT</pubDate>
    </item>
    <item>
      <title>Alert 4</title>
      <link>https://example.com/a4</link>
      <description>Summary 4</description>
      <pubDate>Sat, 09 May 2026 10:03:00 GMT</pubDate>
    </item>
  </channel>
</rss>"#
}

fn rss_with_two_items() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>SEBI</title>
    <link>https://example.com</link>
    <description>SEBI feed</description>
    <item>
      <title>Alert 1</title>
      <link>https://example.com/dry-run-a1</link>
      <description>Summary 1</description>
      <pubDate>Sat, 09 May 2026 10:00:00 GMT</pubDate>
    </item>
    <item>
      <title>Alert 2</title>
      <link>https://example.com/dry-run-a2</link>
      <description>Summary 2</description>
      <pubDate>Sat, 09 May 2026 10:01:00 GMT</pubDate>
    </item>
  </channel>
</rss>"#
}
