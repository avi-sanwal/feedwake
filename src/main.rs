use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "feedwake")]
#[command(about = "Source-aware India market feed notifier for OpenClaw")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Scan {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Scan { config, dry_run } => {
            let summary = feedwake::app::run_scan(config.as_deref(), dry_run)?;
            println!(
                "scan complete: feeds={}, new_items={}, enqueued={}, delivered={}",
                summary.feeds_scanned,
                summary.items_seen,
                summary.events_enqueued,
                summary.events_delivered
            );
        }
    }
    Ok(())
}
