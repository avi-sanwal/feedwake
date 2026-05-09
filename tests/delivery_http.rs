use feedwake::config::OpenClawConfig;
use feedwake::delivery::{OpenClawClient, WakeEvent};
use feedwake::feed::FeedItem;
use serde_json::Value;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Header, Method, Response, Server};

fn event(url: &str, title: &str) -> WakeEvent {
    WakeEvent {
        item: FeedItem {
            source_name: "NSE".to_string(),
            source_url: "https://example.com/feed.xml".to_string(),
            title: title.to_string(),
            url: url.to_string(),
            description: Some("Board Meeting Intimation".to_string()),
            subjects: Vec::new(),
            document_filename: None,
            published_at: None,
        },
        matched_rule: "exchange_watchlist".to_string(),
        matched_entity: Some("RELIANCE".to_string()),
    }
}

#[test]
fn posts_batched_feedwake_event_with_bearer_token_and_article_details() {
    let server = Server::http("127.0.0.1:0").expect("server");
    let address = server.server_addr().to_string();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut request = server.recv().expect("request");
        let auth = request
            .headers()
            .iter()
            .find(|header| header.field.equiv("Authorization"))
            .map(|header| header.value.as_str().to_string());
        let mut body = String::new();
        request.as_reader().read_to_string(&mut body).expect("body");
        assert_eq!(request.method(), &Method::Post);
        request
            .respond(
                Response::from_string("ok")
                    .with_header(Header::from_bytes("Content-Type", "text/plain").expect("header")),
            )
            .expect("response");
        sender.send((auth, body)).expect("send");
    });

    std::env::set_var("OPENCLAW_HOOK_TOKEN", "secret-token");
    let client = OpenClawClient::from_config(
        &OpenClawConfig {
            wake_url: format!("http://{}/hooks/feed-wake", address),
            token_env: "OPENCLAW_HOOK_TOKEN".to_string(),
            token_env_file: None,
            mode: "now".to_string(),
            max_articles_per_wake: 3,
        },
        Duration::from_secs(5),
    )
    .expect("client");

    client
        .post_batch(&[
            event(
                "https://example.com/reliance.pdf",
                "Reliance Industries Limited",
            ),
            event("https://example.com/hdfc.pdf", "HDFC Bank Limited"),
        ])
        .expect("post");
    let (auth, body) = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("posted");
    let payload: Value = serde_json::from_str(&body).expect("json payload");

    assert_eq!(auth.as_deref(), Some("Bearer secret-token"));
    assert_eq!(payload["mode"], "now");
    assert_eq!(payload["articleCount"], 2);
    assert!(payload["text"]
        .as_str()
        .expect("text")
        .contains("Description: Board Meeting Intimation"));
    assert_eq!(payload["articles"][0]["source"], "NSE");
    assert_eq!(
        payload["articles"][0]["sourceUrl"],
        "https://example.com/feed.xml"
    );
    assert_eq!(
        payload["articles"][1]["url"],
        "https://example.com/hdfc.pdf"
    );
    assert!(payload.get("sessionKey").is_none());
}

#[test]
fn posts_batched_feedwake_event_with_request_session_key() {
    let server = Server::http("127.0.0.1:0").expect("server");
    let address = server.server_addr().to_string();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut request = server.recv().expect("request");
        let mut body = String::new();
        request.as_reader().read_to_string(&mut body).expect("body");
        request
            .respond(Response::from_string("ok"))
            .expect("response");
        sender.send(body).expect("send");
    });

    std::env::set_var("OPENCLAW_HOOK_TOKEN", "secret-token");
    let client = OpenClawClient::from_config(
        &OpenClawConfig {
            wake_url: format!("http://{}/hooks/feed-wake", address),
            token_env: "OPENCLAW_HOOK_TOKEN".to_string(),
            token_env_file: None,
            mode: "now".to_string(),
            max_articles_per_wake: 3,
        },
        Duration::from_secs(5),
    )
    .expect("client");

    client
        .post_batch_with_session_key(
            &[event(
                "https://example.com/reliance.pdf",
                "Reliance Industries Limited",
            )],
            Some("hook:feedwake:test-batch"),
        )
        .expect("post");

    let body = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("posted");
    let payload: Value = serde_json::from_str(&body).expect("json payload");

    assert_eq!(payload["sessionKey"], "hook:feedwake:test-batch");
}

#[test]
fn non_success_status_is_delivery_error() {
    let server = Server::http("127.0.0.1:0").expect("server");
    let address = server.server_addr().to_string();
    thread::spawn(move || {
        let request = server.recv().expect("request");
        request
            .respond(Response::from_string("fail").with_status_code(500))
            .expect("response");
    });

    std::env::set_var("OPENCLAW_HOOK_TOKEN", "secret-token");
    let client = OpenClawClient::from_config(
        &OpenClawConfig {
            wake_url: format!("http://{}/hooks/feed-wake", address),
            token_env: "OPENCLAW_HOOK_TOKEN".to_string(),
            token_env_file: None,
            mode: "now".to_string(),
            max_articles_per_wake: 3,
        },
        Duration::from_secs(5),
    )
    .expect("client");

    assert!(client
        .post_batch(&[event(
            "https://example.com/reliance.pdf",
            "Reliance Industries Limited"
        )])
        .is_err());
}

#[test]
fn reads_bearer_token_from_openclaw_env_file_when_env_is_missing() {
    std::env::remove_var("FEEDWAKE_TEST_HOOK_TOKEN_FILE");
    let temp_dir = TempDir::new().expect("temp dir");
    let env_file = temp_dir.path().join(".env");
    fs::write(
        &env_file,
        "export FEEDWAKE_TEST_HOOK_TOKEN_FILE='file-token'\n",
    )
    .expect("write env file");

    let server = Server::http("127.0.0.1:0").expect("server");
    let address = server.server_addr().to_string();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let request = server.recv().expect("request");
        let auth = request
            .headers()
            .iter()
            .find(|header| header.field.equiv("Authorization"))
            .map(|header| header.value.as_str().to_string());
        request
            .respond(Response::from_string("ok"))
            .expect("response");
        sender.send(auth).expect("send");
    });

    let client = OpenClawClient::from_config(
        &OpenClawConfig {
            wake_url: format!("http://{}/hooks/feed-wake", address),
            token_env: "FEEDWAKE_TEST_HOOK_TOKEN_FILE".to_string(),
            token_env_file: Some(env_file.to_string_lossy().to_string()),
            mode: "now".to_string(),
            max_articles_per_wake: 3,
        },
        Duration::from_secs(5),
    )
    .expect("client");

    client
        .post(&event(
            "https://example.com/reliance.pdf",
            "Reliance Industries Limited",
        ))
        .expect("post");

    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("posted")
            .as_deref(),
        Some("Bearer file-token")
    );
}
