use anyhow::{anyhow, Result};
use chrono::{SecondsFormat, Utc};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::config::{default_state_db_path, load_config};
use crate::delivery::{OpenClawClient, WakeEvent};
use crate::feed::scan_feed;
use crate::filter::evaluate_item;
use crate::state::StateStore;

static FILE_LOGGER: OnceLock<Mutex<File>> = OnceLock::new();

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
    let state = if options.dry_run {
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
            log_stderr(format!("scanning feed: {} ({})", feed.name, feed.url));
        }
        let items = match scan_feed(feed, &config.scan, &state) {
            Ok(items) => items,
            Err(error) => {
                summary.feed_errors += 1;
                log_stderr(format!("feed error for {}: {}", feed.name, error));
                continue;
            }
        };
        if options.verbose {
            log_stderr(format!(
                "feed items fetched: {} ({})",
                items.len(),
                feed.name
            ));
        }

        for item in items {
            if options.verbose {
                log_stderr(format!(
                    "feed item found: feed=\"{}\" title=\"{}\" url={}",
                    feed.name,
                    compact_log_value(&item.title),
                    item.url
                ));
            }
            if state.has_seen_url(&item.url)? {
                if options.verbose {
                    log_stderr(format!(
                        "feed item skipped: feed=\"{}\" title=\"{}\" reason=already_seen url={}",
                        feed.name,
                        compact_log_value(&item.title),
                        item.url
                    ));
                }
                continue;
            }
            summary.items_seen += 1;
            let decision = evaluate_item(&config, feed.filter_profile, &item);
            state.mark_seen_url(&item.url)?;
            if !decision.matched {
                if options.verbose {
                    log_stderr(format!(
                        "feed item discarded: feed=\"{}\" title=\"{}\" reason={} url={}",
                        feed.name,
                        compact_log_value(&item.title),
                        decision.reason,
                        item.url
                    ));
                }
                continue;
            }
            let event = WakeEvent {
                item,
                matched_rule: decision.reason,
                matched_entity: decision.matched_entity,
            };
            if options.dry_run {
                if options.verbose {
                    log_stderr(format!(
                        "feed item matched: feed=\"{}\" title=\"{}\" reason={} entity={} action=dry_run url={}",
                        feed.name,
                        compact_log_value(&event.item.title),
                        event.matched_rule,
                        event.matched_entity.as_deref().unwrap_or("-"),
                        event.item.url
                    ));
                }
                log_stdout(format!("dry-run match: {}", event.wake_text()));
            } else {
                if options.verbose {
                    log_stderr(format!(
                        "feed item matched: feed=\"{}\" title=\"{}\" reason={} entity={} action=queued url={}",
                        feed.name,
                        compact_log_value(&event.item.title),
                        event.matched_rule,
                        event.matched_entity.as_deref().unwrap_or("-"),
                        event.item.url
                    ));
                }
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
            log_stderr("delivery queue empty");
        }
        return Ok(());
    }
    let client = OpenClawClient::from_config(openclaw, Duration::from_secs(timeout_seconds))?;
    let ids: Vec<_> = pending.iter().map(|(id, _)| *id).collect();
    let events: Vec<_> = pending.iter().map(|(_, event)| event.clone()).collect();
    if verbose {
        log_stderr(format!("delivering pending events: {}", pending.len()));
    }
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
            log_stderr(format!("delivery error for FeedWake batch: {}", error));
        }
    }
    Ok(())
}

pub fn log_stdout(message: impl AsRef<str>) {
    write_log_line(timestamped(message.as_ref()), OutputStream::Stdout);
}

pub fn log_stderr(message: impl AsRef<str>) {
    write_log_line(timestamped(message.as_ref()), OutputStream::Stderr);
}

pub fn configure_file_logging(log_file: &Path, max_bytes: u64, rotate_count: u8) -> Result<()> {
    validate_log_rotation(max_bytes, rotate_count)?;
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }
    rotate_log_file(log_file, max_bytes, rotate_count)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)?;
    FILE_LOGGER
        .set(Mutex::new(file))
        .map_err(|_| anyhow!("file logging has already been configured"))
}

fn write_log_line(line: String, stream: OutputStream) {
    if let Some(logger) = FILE_LOGGER.get() {
        if let Ok(mut file) = logger.lock() {
            let _ = writeln!(file, "{line}");
            return;
        }
    }

    match stream {
        OutputStream::Stdout => println!("{line}"),
        OutputStream::Stderr => eprintln!("{line}"),
    }
}

fn rotate_log_file(log_file: &Path, max_bytes: u64, rotate_count: u8) -> Result<()> {
    let should_rotate = match fs::metadata(log_file) {
        Ok(metadata) => metadata.len() >= max_bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if !should_rotate {
        return Ok(());
    }

    for index in (1..rotate_count).rev() {
        let source = rotated_log_path(log_file, index);
        let destination = rotated_log_path(log_file, index + 1);
        match fs::rename(&source, &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    if rotate_count > 0 {
        fs::rename(log_file, rotated_log_path(log_file, 1))?;
    } else {
        File::create(log_file)?;
    }
    Ok(())
}

fn rotated_log_path(log_file: &Path, index: u8) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.{}", log_file.display(), index))
}

fn validate_log_rotation(log_max_bytes: u64, log_rotate_count: u8) -> Result<()> {
    if log_max_bytes == 0 {
        return Err(anyhow!("log max bytes must be greater than 0"));
    }
    if log_rotate_count > 30 {
        return Err(anyhow!("log rotate count must be between 0 and 30"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputStream {
    Stdout,
    Stderr,
}

fn timestamped(message: &str) -> String {
    format!(
        "[{}] {}",
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        message
    )
}

fn compact_log_value(value: &str) -> String {
    let single_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= 160 {
        single_line
    } else {
        format!("{}...", single_line.chars().take(157).collect::<String>())
    }
}
