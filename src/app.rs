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

pub fn run_scan(config_path: Option<&Path>, dry_run: bool) -> Result<ScanSummary> {
    let (config, _) = load_config(config_path)?;
    let state = if dry_run && config.scan.state_db.is_none() {
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
        let items = match scan_feed(feed, &config.scan, &state) {
            Ok(items) => items,
            Err(error) => {
                summary.feed_errors += 1;
                eprintln!("feed error for {}: {}", feed.name, error);
                continue;
            }
        };

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
            if dry_run {
                println!("dry-run match: {}", event.wake_text());
            } else {
                state.enqueue_event(&event)?;
            }
            summary.events_enqueued += 1;
        }
    }

    if !dry_run {
        deliver_pending(
            &config.openclaw,
            config.scan.timeout_seconds,
            &state,
            &mut summary,
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
) -> Result<()> {
    let pending = state.pending_events()?;
    if pending.is_empty() {
        return Ok(());
    }

    let client = OpenClawClient::from_config(openclaw, Duration::from_secs(timeout_seconds))?;
    for (id, event) in pending {
        match client.post(&event) {
            Ok(()) => {
                state.mark_delivered(id)?;
                summary.events_delivered += 1;
            }
            Err(error) => {
                summary.delivery_errors += 1;
                state.mark_delivery_failed(id, &error.to_string())?;
                eprintln!("delivery error for {}: {}", event.item.url, error);
            }
        }
    }
    Ok(())
}
