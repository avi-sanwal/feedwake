use feedwake::config::{
    Config, ExchangeFilters, FeedConfig, FilterProfile, MediaFilters, OpenClawConfig, ScanConfig,
    SourceType, WatchlistEntry,
};
use feedwake::feed::FeedItem;
use feedwake::filter::evaluate_item;

fn test_config() -> Config {
    Config {
        openclaw: OpenClawConfig {
            wake_url: "http://127.0.0.1:18789/hooks/feed-wake".to_string(),
            token_env: "OPENCLAW_HOOK_TOKEN".to_string(),
            mode: "now".to_string(),
            max_articles_per_wake: 3,
        },
        scan: ScanConfig {
            timeout_seconds: 10,
            max_items_per_feed: 30,
            max_response_bytes: 1_048_576,
            conditional_get: true,
            state_db: None,
        },
        watchlist: vec![WatchlistEntry {
            symbol: "RELIANCE".to_string(),
            name: "Reliance Industries Limited".to_string(),
            isin: Some("INE002A01018".to_string()),
            aliases: vec!["Reliance Industries".to_string(), "RIL".to_string()],
        }],
        feeds: vec![FeedConfig {
            name: "NSE Announcements".to_string(),
            url: "https://nsearchives.nseindia.com/content/RSS/Online_announcements.xml"
                .to_string(),
            source_type: SourceType::Nse,
            filter_profile: FilterProfile::ExchangeWatchlist,
            user_agent: None,
        }],
        exchange_filters: ExchangeFilters {
            category_allowlist: vec!["Board Meeting".to_string(), "Financial Results".to_string()],
        },
        media_filters: MediaFilters {
            require_watchlist_match: true,
            keyword_groups: vec!["results".to_string(), "rating downgrade".to_string()],
            exclude_keywords: vec!["sponsored".to_string(), "personal finance".to_string()],
        },
    }
}

fn item(title: &str, description: &str) -> FeedItem {
    FeedItem {
        source_name: "source".to_string(),
        source_url: "https://example.com/feed.xml".to_string(),
        title: title.to_string(),
        url: "https://example.com/item".to_string(),
        description: Some(description.to_string()),
        subjects: Vec::new(),
        document_filename: None,
        published_at: None,
    }
}

#[test]
fn exchange_watchlist_matches_symbol_alias_and_isin() {
    let config = test_config();
    let symbol = item(
        "Reliance Industries Limited",
        "RELIANCE announces board meeting",
    );
    let alias = item("RIL update", "Board Meeting Intimation");
    let isin = item("Issuer update", "INE002A01018 Financial Results");

    assert!(evaluate_item(&config, FilterProfile::ExchangeWatchlist, &symbol).matched);
    assert!(evaluate_item(&config, FilterProfile::ExchangeWatchlist, &alias).matched);
    assert!(evaluate_item(&config, FilterProfile::ExchangeWatchlist, &isin).matched);
}

#[test]
fn exchange_watchlist_does_not_match_symbol_inside_another_word() {
    let config = test_config();
    let unrelated = item("Unrelated issuer", "PRELIANCE vendor update");

    assert!(!evaluate_item(&config, FilterProfile::ExchangeWatchlist, &unrelated).matched);
}

#[test]
fn exchange_watchlist_matches_subject_allowlist_and_document_filename() {
    let config = test_config();
    let mut item = item("Corporate announcement", "Exchange filing");
    item.subjects = vec!["Financial Results".to_string()];
    item.document_filename = Some("RELIANCE_Q4_Results.pdf".to_string());

    let decision = evaluate_item(&config, FilterProfile::ExchangeWatchlist, &item);

    assert!(decision.matched);
    assert_eq!(decision.matched_entity.as_deref(), Some("RELIANCE"));
}

#[test]
fn bse_watchlist_matches_title_or_description_when_subject_is_allowed() {
    let config = test_config();
    let mut title_match = item("Reliance Industries Limited", "Exchange filing");
    title_match.source_name = "BSE Corporate Announcements".to_string();
    title_match.subjects = vec!["Financial Results".to_string()];

    let mut description_match = item("Corporate announcement", "RIL board meeting intimation");
    description_match.source_name = "BSE Corporate Announcements".to_string();
    description_match.subjects = vec!["Board Meeting".to_string()];

    assert!(evaluate_item(&config, FilterProfile::ExchangeWatchlist, &title_match).matched);
    assert!(
        evaluate_item(
            &config,
            FilterProfile::ExchangeWatchlist,
            &description_match
        )
        .matched
    );
}

#[test]
fn bse_watchlist_rejects_unrelated_company_even_with_allowed_subject() {
    let config = test_config();
    let mut item = item("Corporate announcement", "Exchange filing");
    item.source_name = "BSE Corporate Announcements".to_string();
    item.source_url = "https://www.bseindia.com/rss-feed.html".to_string();
    item.url =
        "https://www.bseindia.com/xml-data/corpfiling/AttachLive/TCS_Q4_Results.pdf".to_string();
    item.subjects = vec!["Financial Results".to_string()];
    item.document_filename = Some("TCS_Q4_Results.pdf".to_string());

    let decision = evaluate_item(&config, FilterProfile::ExchangeWatchlist, &item);

    assert!(!decision.matched);
    assert_eq!(decision.reason, "watchlist_miss");
}

#[test]
fn authority_passthrough_matches_every_new_item() {
    let config = test_config();
    let decision = evaluate_item(
        &config,
        FilterProfile::AuthorityPassthrough,
        &item("RBI update", "No watched entity here"),
    );

    assert!(decision.matched);
    assert_eq!(decision.reason, "authority_passthrough");
}

#[test]
fn media_high_precision_requires_watchlist_and_market_keyword() {
    let config = test_config();

    let matched = evaluate_item(
        &config,
        FilterProfile::MediaHighPrecision,
        &item(
            "Reliance Industries results beat estimates",
            "market reaction",
        ),
    );
    let entity_only = evaluate_item(
        &config,
        FilterProfile::MediaHighPrecision,
        &item("Reliance Industries profile", "how the company was founded"),
    );
    let keyword_only = evaluate_item(
        &config,
        FilterProfile::MediaHighPrecision,
        &item("Bank results beat estimates", "no watchlist entity"),
    );

    assert!(matched.matched);
    assert!(!entity_only.matched);
    assert!(!keyword_only.matched);
}

#[test]
fn media_high_precision_honors_exclude_keywords() {
    let config = test_config();
    let decision = evaluate_item(
        &config,
        FilterProfile::MediaHighPrecision,
        &item(
            "Reliance Industries results sponsored package",
            "rating downgrade mentioned in sponsored content",
        ),
    );

    assert!(!decision.matched);
    assert_eq!(decision.reason, "excluded_keyword");
}
