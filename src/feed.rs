use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use feed_rs::model::Entry;
use percent_encoding::percent_decode_str;
use std::io::Read;
use std::time::Duration;
use url::Url;

use crate::config::{FeedConfig, ScanConfig, SourceType};
use crate::state::{FeedCache, StateStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedItem {
    pub source_name: String,
    pub source_url: String,
    pub title: String,
    pub url: String,
    pub description: Option<String>,
    pub subjects: Vec<String>,
    pub document_filename: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

impl FeedItem {
    pub fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.title,
            self.description.as_deref().unwrap_or_default(),
            self.subjects.join(" "),
            self.url,
            self.document_filename.as_deref().unwrap_or_default(),
            self.source_name
        )
    }
}

pub struct FeedFetchResult {
    pub items: Vec<FeedItem>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub not_modified: bool,
}

pub fn fetch_feed(
    feed: &FeedConfig,
    scan: &ScanConfig,
    cache: Option<&FeedCache>,
) -> Result<FeedFetchResult> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(scan.timeout_seconds))
        .build();
    let mut request = agent.get(&feed.url);

    request = request.set("User-Agent", user_agent_for_feed(feed));

    if scan.conditional_get {
        if let Some(cache) = cache {
            if let Some(etag) = &cache.etag {
                request = request.set("If-None-Match", etag);
            }
            if let Some(last_modified) = &cache.last_modified {
                request = request.set("If-Modified-Since", last_modified);
            }
        }
    }

    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(304, response)) => {
            return Ok(FeedFetchResult {
                items: Vec::new(),
                etag: response.header("ETag").map(ToOwned::to_owned),
                last_modified: response.header("Last-Modified").map(ToOwned::to_owned),
                not_modified: true,
            });
        }
        Err(error) => return Err(anyhow!("failed to fetch {}: {}", feed.url, error)),
    };

    if response.status() < 200 || response.status() >= 300 {
        if response.status() == 304 {
            return Ok(FeedFetchResult {
                items: Vec::new(),
                etag: response.header("ETag").map(ToOwned::to_owned),
                last_modified: response.header("Last-Modified").map(ToOwned::to_owned),
                not_modified: true,
            });
        }
        return Err(anyhow!(
            "failed to fetch {}: status {}",
            feed.url,
            response.status()
        ));
    }

    let etag = response.header("ETag").map(ToOwned::to_owned);
    let last_modified = response.header("Last-Modified").map(ToOwned::to_owned);
    let mut body = Vec::new();
    response
        .into_reader()
        .take((scan.max_response_bytes as u64).saturating_add(1))
        .read_to_end(&mut body)
        .with_context(|| format!("failed to read {}", feed.url))?;
    if body.len() > scan.max_response_bytes {
        return Err(anyhow!(
            "feed response exceeded max_response_bytes for {}",
            feed.url
        ));
    }

    Ok(FeedFetchResult {
        items: parse_feed_bytes(&body, &feed.name, &feed.url, scan.max_items_per_feed)
            .with_context(|| format!("failed to parse {}", feed.url))?,
        etag,
        last_modified,
        not_modified: false,
    })
}

pub fn parse_feed_bytes(
    body: &[u8],
    source_name: &str,
    source_url: &str,
    max_items: usize,
) -> Result<Vec<FeedItem>> {
    let parsed = feed_rs::parser::parse(body)?;
    Ok(parsed
        .entries
        .iter()
        .take(max_items)
        .filter_map(|entry| entry_to_item(entry, source_name, source_url))
        .collect())
}

pub fn scan_feed(
    feed: &FeedConfig,
    scan: &ScanConfig,
    state: &StateStore,
) -> Result<Vec<FeedItem>> {
    let cache = state.feed_cache(&feed.url)?;
    let result = fetch_feed(feed, scan, cache.as_ref())?;
    if !result.not_modified {
        state.update_feed_cache(
            &feed.url,
            result.etag.as_deref(),
            result.last_modified.as_deref(),
        )?;
    }
    Ok(result.items)
}

fn entry_to_item(entry: &Entry, source_name: &str, source_url: &str) -> Option<FeedItem> {
    let title = entry.title.as_ref()?.content.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let url = entry
        .links
        .iter()
        .find(|link| link.rel.as_deref().unwrap_or("alternate") == "alternate")
        .or_else(|| entry.links.first())
        .map(|link| link.href.trim().to_string())
        .filter(|href| !href.is_empty())?;

    let document_filename = document_filename(&url);

    Some(FeedItem {
        source_name: source_name.to_string(),
        source_url: source_url.to_string(),
        title,
        url,
        description: entry
            .summary
            .as_ref()
            .map(|summary| summary.content.clone()),
        subjects: entry_subjects(entry),
        document_filename,
        published_at: entry.published.or(entry.updated),
    })
}

fn user_agent_for_feed(feed: &FeedConfig) -> &str {
    feed.user_agent
        .as_deref()
        .unwrap_or(match feed.source_type {
            SourceType::Bse => "Mozilla/5.0 (compatible; FeedWake/0.1)",
            _ => "FeedWake/0.1",
        })
}

fn entry_subjects(entry: &Entry) -> Vec<String> {
    entry
        .categories
        .iter()
        .flat_map(|category| {
            [
                category.term.trim(),
                category.label.as_deref().unwrap_or_default().trim(),
            ]
        })
        .filter(|subject| !subject.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn document_filename(url: &str) -> Option<String> {
    let path_segment = Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back().map(ToOwned::to_owned))
        })
        .or_else(|| url.rsplit('/').next().map(ToOwned::to_owned))?;

    let trimmed = path_segment.trim();
    if trimmed.is_empty() {
        return None;
    }

    let decoded = percent_decode_str(trimmed).decode_utf8_lossy();
    let filename = decoded.trim();
    if filename.is_empty() {
        None
    } else {
        Some(filename.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::user_agent_for_feed;
    use crate::config::{FeedConfig, FilterProfile, SourceType};

    #[test]
    fn bse_uses_browser_compatible_default_user_agent() {
        let feed = FeedConfig {
            name: "BSE Corporate Announcements".to_string(),
            url: "https://www.bseindia.com/rss-feed.html".to_string(),
            source_type: SourceType::Bse,
            filter_profile: FilterProfile::ExchangeWatchlist,
            user_agent: None,
        };

        assert!(user_agent_for_feed(&feed).contains("Mozilla/5.0"));
    }

    #[test]
    fn configured_user_agent_overrides_bse_default() {
        let feed = FeedConfig {
            name: "BSE Corporate Announcements".to_string(),
            url: "https://www.bseindia.com/rss-feed.html".to_string(),
            source_type: SourceType::Bse,
            filter_profile: FilterProfile::ExchangeWatchlist,
            user_agent: Some("custom-agent".to_string()),
        };

        assert_eq!(user_agent_for_feed(&feed), "custom-agent");
    }
}
