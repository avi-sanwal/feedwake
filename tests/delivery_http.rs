use feedwake::config::OpenClawConfig;
use feedwake::delivery::{OpenClawClient, WakeEvent};
use feedwake::feed::FeedItem;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Response, Server};

fn event() -> WakeEvent {
    WakeEvent {
        item: FeedItem {
            source_name: "NSE".to_string(),
            source_url: "https://example.com/feed.xml".to_string(),
            title: "Reliance Industries Limited".to_string(),
            url: "https://example.com/reliance.pdf".to_string(),
            description: Some("Board Meeting Intimation".to_string()),
            published_at: None,
        },
        matched_rule: "exchange_watchlist".to_string(),
        matched_entity: Some("RELIANCE".to_string()),
    }
}

#[test]
fn posts_wake_event_with_bearer_token() {
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
            mode: "now".to_string(),
        },
        Duration::from_secs(5),
    )
    .expect("client");

    client.post(&event()).expect("post");
    let (auth, body) = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("posted");

    assert_eq!(auth.as_deref(), Some("Bearer secret-token"));
    assert!(body.contains("Feed alert: NSE published"));
    assert!(body.contains("RELIANCE"));
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
            mode: "now".to_string(),
        },
        Duration::from_secs(5),
    )
    .expect("client");

    assert!(client.post(&event()).is_err());
}
