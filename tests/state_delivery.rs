use feedwake::delivery::WakeEvent;
use feedwake::feed::FeedItem;
use feedwake::state::{DeliveryStatus, StateStore};
use tempfile::NamedTempFile;

fn item(url: &str) -> FeedItem {
    FeedItem {
        source_name: "NSE".to_string(),
        source_url: "https://example.com/feed.xml".to_string(),
        title: "Reliance Industries Limited".to_string(),
        url: url.to_string(),
        description: Some("Board Meeting Intimation".to_string()),
        published_at: None,
    }
}

#[test]
fn state_store_deduplicates_seen_urls() {
    let db = NamedTempFile::new().expect("temp db");
    let store = StateStore::open(db.path()).expect("open state");

    assert!(!store
        .has_seen_url("https://example.com/a")
        .expect("seen check"));
    store
        .mark_seen_url("https://example.com/a")
        .expect("mark seen");
    assert!(store
        .has_seen_url("https://example.com/a")
        .expect("seen check"));
}

#[test]
fn outbox_marks_delivered_only_after_success() {
    let db = NamedTempFile::new().expect("temp db");
    let store = StateStore::open(db.path()).expect("open state");
    let event = WakeEvent {
        item: item("https://example.com/a"),
        matched_rule: "exchange_watchlist".to_string(),
        matched_entity: Some("RELIANCE".to_string()),
    };

    let id = store.enqueue_event(&event).expect("enqueue event");
    assert_eq!(
        store.event_status(id).expect("status"),
        DeliveryStatus::Pending
    );

    store.mark_delivered(id).expect("mark delivered");
    assert_eq!(
        store.event_status(id).expect("status"),
        DeliveryStatus::Delivered
    );
}

#[test]
fn pending_events_limit_returns_oldest_events_without_marking_excess_delivered() {
    let db = NamedTempFile::new().expect("temp db");
    let store = StateStore::open(db.path()).expect("open state");
    let first = store
        .enqueue_event(&WakeEvent {
            item: item("https://example.com/a"),
            matched_rule: "exchange_watchlist".to_string(),
            matched_entity: Some("RELIANCE".to_string()),
        })
        .expect("enqueue first");
    let second = store
        .enqueue_event(&WakeEvent {
            item: item("https://example.com/b"),
            matched_rule: "exchange_watchlist".to_string(),
            matched_entity: Some("RELIANCE".to_string()),
        })
        .expect("enqueue second");
    let third = store
        .enqueue_event(&WakeEvent {
            item: item("https://example.com/c"),
            matched_rule: "exchange_watchlist".to_string(),
            matched_entity: Some("RELIANCE".to_string()),
        })
        .expect("enqueue third");

    let pending = store.pending_events_limit(2).expect("pending limit");

    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].0, first);
    assert_eq!(pending[1].0, second);
    assert_eq!(
        store.event_status(third).expect("third status"),
        DeliveryStatus::Pending
    );
}
