use regex::Regex;

use crate::config::{Config, FilterProfile, WatchlistEntry};
use crate::feed::FeedItem;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchDecision {
    pub matched: bool,
    pub reason: String,
    pub matched_entity: Option<String>,
}

pub fn evaluate_item(config: &Config, profile: FilterProfile, item: &FeedItem) -> MatchDecision {
    match profile {
        FilterProfile::ExchangeWatchlist => evaluate_exchange(config, item),
        FilterProfile::AuthorityPassthrough => MatchDecision {
            matched: true,
            reason: "authority_passthrough".to_string(),
            matched_entity: None,
        },
        FilterProfile::MediaHighPrecision => evaluate_media(config, item),
    }
}

fn evaluate_exchange(config: &Config, item: &FeedItem) -> MatchDecision {
    let text = item.searchable_text();
    let matched_entity = find_entity(&config.watchlist, &text);
    if matched_entity.is_none() {
        return no_match("watchlist_miss");
    }

    if !config.exchange_filters.category_allowlist.is_empty()
        && !matches_any_phrase(&text, &config.exchange_filters.category_allowlist)
    {
        return no_match("category_miss");
    }

    MatchDecision {
        matched: true,
        reason: "exchange_watchlist".to_string(),
        matched_entity,
    }
}

fn evaluate_media(config: &Config, item: &FeedItem) -> MatchDecision {
    let text = item.searchable_text();
    if matches_any_phrase(&text, &config.media_filters.exclude_keywords) {
        return no_match("excluded_keyword");
    }

    let matched_entity = find_entity(&config.watchlist, &text);
    if config.media_filters.require_watchlist_match && matched_entity.is_none() {
        return no_match("watchlist_miss");
    }

    if !matches_any_phrase(&text, &config.media_filters.keyword_groups) {
        return no_match("keyword_miss");
    }

    MatchDecision {
        matched: true,
        reason: "media_high_precision".to_string(),
        matched_entity,
    }
}

fn no_match(reason: &str) -> MatchDecision {
    MatchDecision {
        matched: false,
        reason: reason.to_string(),
        matched_entity: None,
    }
}

fn find_entity(watchlist: &[WatchlistEntry], text: &str) -> Option<String> {
    for entry in watchlist {
        for term in entity_terms(entry) {
            if matches_term(text, &term) {
                return Some(entry.symbol.clone());
            }
        }
    }
    None
}

fn entity_terms(entry: &WatchlistEntry) -> Vec<String> {
    let mut terms = Vec::new();
    terms.push(entry.symbol.clone());
    terms.push(entry.name.clone());
    if let Some(isin) = &entry.isin {
        terms.push(isin.clone());
    }
    terms.extend(entry.aliases.clone());
    terms
}

fn matches_any_phrase(text: &str, phrases: &[String]) -> bool {
    phrases.iter().any(|phrase| matches_term(text, phrase))
}

fn matches_term(text: &str, term: &str) -> bool {
    let trimmed = term.trim();
    if trimmed.is_empty() {
        return false;
    }
    let pattern = format!(
        r"(?i)(^|[^A-Za-z0-9]){}([^A-Za-z0-9]|$)",
        regex::escape(trimmed)
    );
    Regex::new(&pattern)
        .map(|regex| regex.is_match(text))
        .unwrap_or(false)
}
