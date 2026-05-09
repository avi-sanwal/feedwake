use anyhow::{anyhow, Result};
use std::path::Path;
use std::time::Duration;

use crate::config::{default_state_db_path, load_config};
use crate::delivery::{OpenClawClient, WakeEvent};
use crate::feed::scan_feed;
use crate::filter::evaluate_item;
use crate::state::StateStore;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanSummary {
    pub feeds_scanned: usize,
    pub items_seen: usize,
    pub events_enqueued: usize,
    pub events_delivered: usize,
    pub feed_errors: usize,
    pub delivery_errors: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanOptions {
    pub dry_run: bool,
    pub verbose: bool,
}

pub fn run_scan(config_path: Option<&Path>, dry_run: bool) -> Result<ScanSummary> {
    run_scan_with_options(
        config_path,
        ScanOptions {
            dry_run,
            verbose: false,
        },
    )
}

pub fn run_scan_with_options(
    config_path: Option<&Path>,
    options: ScanOptions,
) -> Result<ScanSummary> {
    let (config, _) = load_config(config_path)?;
    let state = if options.dry_run && config.scan.state_db.is_none() {
        StateStore::memory()?
    } else {
        let db_path = config
            .scan
            .state_db
            .as_ref()
            .map(Path::new)
            .map(ToOwned::to_owned)
            .unwrap_or_else(default_state_db_path);
        StateStore::open(&db_path)?
    };

    let mut summary = ScanSummary::default();
    for feed in &config.feeds {
        summary.feeds_scanned += 1;
        if options.verbose {
            eprintln!("scanning feed: {} ({})", feed.name, feed.url);
        }
        let items = match scan_feed(feed, &config.scan, &state) {
            Ok(items) => items,
            Err(error) => {
                summary.feed_errors += 1;
                eprintln!("feed error for {}: {}", feed.name, error);
                continue;
            }
        };
        if options.verbose {
            eprintln!("feed items fetched: {} ({})", items.len(), feed.name);
        }

        for item in items {
            if state.has_seen_url(&item.url)? {
                continue;
            }
            summary.items_seen += 1;
            let decision = evaluate_item(&config, feed.filter_profile, &item);
            state.mark_seen_url(&item.url)?;
            if !decision.matched {
                continue;
            }
            let event = WakeEvent {
                item,
                matched_rule: decision.reason,
                matched_entity: decision.matched_entity,
            };
            if options.dry_run {
                println!("dry-run match: {}", event.wake_text());
            } else {
                state.enqueue_event(&event)?;
            }
            summary.events_enqueued += 1;
        }
    }

    if !options.dry_run {
        deliver_pending(
            &config.openclaw,
            config.scan.timeout_seconds,
            &state,
            &mut summary,
            options.verbose,
        )?;
    }

    if summary.feed_errors > 0 || summary.delivery_errors > 0 {
        return Err(anyhow!(
            "scan completed with {} feed error(s) and {} delivery error(s)",
            summary.feed_errors,
            summary.delivery_errors
        ));
    }

    Ok(summary)
}

fn deliver_pending(
    openclaw: &crate::config::OpenClawConfig,
    timeout_seconds: u64,
    state: &StateStore,
    summary: &mut ScanSummary,
    verbose: bool,
) -> Result<()> {
    if openclaw.max_articles_per_wake == 0 {
        return Err(anyhow!(
            "openclaw.max_articles_per_wake must be greater than 0"
        ));
    }

    let pending = state.pending_events_limit(openclaw.max_articles_per_wake)?;
    if pending.is_empty() {
        if verbose {
            eprintln!("delivery queue empty");
        }
        return Ok(());
    }
    if verbose {
        eprintln!("delivering pending events: {}", pending.len());
    }

    let client = OpenClawClient::from_config(openclaw, Duration::from_secs(timeout_seconds))?;
    let ids: Vec<_> = pending.iter().map(|(id, _)| *id).collect();
    let events: Vec<_> = pending.iter().map(|(_, event)| event.clone()).collect();
    match client.post_batch(&events) {
        Ok(()) => {
            for id in ids {
                state.mark_delivered(id)?;
                summary.events_delivered += 1;
            }
        }
        Err(error) => {
            summary.delivery_errors += events.len();
            for id in ids {
                state.mark_delivery_failed(id, &error.to_string())?;
            }
            eprintln!("delivery error for FeedWake batch: {}", error);
        }
    }
    Ok(())
}
