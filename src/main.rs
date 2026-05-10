use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "feedwake")]
#[command(about = "Source-aware India market feed notifier for OpenClaw")]
#[command(version)]
struct Cli {
    #[arg(long, short, global = true)]
    verbose: bool,
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
        #[arg(long)]
        log_file: Option<PathBuf>,
        #[arg(long, default_value_t = 1_048_576)]
        log_max_bytes: u64,
        #[arg(long, default_value_t = 5)]
        log_rotate_count: u8,
    },
    Openclaw {
        #[command(subcommand)]
        command: OpenClawCommands,
    },
}

#[derive(Debug, Subcommand)]
enum OpenClawCommands {
    Install {
        #[arg(long)]
        openclaw_config_dir: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, default_value_t = 5)]
        frequency_minutes: u8,
        #[arg(long)]
        feedwake_bin: Option<PathBuf>,
        #[arg(long, default_value = feedwake::openclaw::DEFAULT_HOOK_TOKEN_ENV)]
        hook_token_env: String,
        #[arg(long)]
        log_file: Option<PathBuf>,
        #[arg(long, default_value_t = 1_048_576)]
        log_max_bytes: u64,
        #[arg(long, default_value_t = 5)]
        log_rotate_count: u8,
        #[arg(long)]
        message_template: Option<String>,
    },
}

fn main() {
    if let Err(error) = run() {
        feedwake::app::log_stderr(format!("Error: {error:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Scan {
            config,
            dry_run,
            log_file,
            log_max_bytes,
            log_rotate_count,
        } => {
            if let Some(log_file) = log_file.as_deref() {
                feedwake::app::configure_file_logging(log_file, log_max_bytes, log_rotate_count)?;
            }
            let summary = feedwake::app::run_scan_with_options(
                config.as_deref(),
                feedwake::app::ScanOptions {
                    dry_run,
                    verbose: cli.verbose,
                },
            )?;
            feedwake::app::log_stdout(format!(
                "scan complete: feeds={}, new_items={}, enqueued={}, delivered={}",
                summary.feeds_scanned,
                summary.items_seen,
                summary.events_enqueued,
                summary.events_delivered
            ));
        }
        Commands::Openclaw { command } => match command {
            OpenClawCommands::Install {
                openclaw_config_dir,
                config,
                frequency_minutes,
                feedwake_bin,
                hook_token_env,
                log_file,
                log_max_bytes,
                log_rotate_count,
                message_template,
            } => {
                let summary = feedwake::openclaw::install_openclaw(
                    feedwake::openclaw::OpenClawInstallRequest {
                        openclaw_config_dir,
                        feedwake_config_path: config,
                        feedwake_bin,
                        frequency_minutes,
                        hook_token_env,
                        log_file,
                        log_max_bytes,
                        log_rotate_count,
                        message_template,
                    },
                )?;
                feedwake::app::log_stdout(format!(
                    "openclaw install complete: config={}, feedwake_config={}, feedwake_bin={}, frequency_minutes={}, log_file={}",
                    summary.openclaw_config_path.display(),
                    summary.feedwake_config_path.display(),
                    summary.feedwake_bin.display(),
                    summary.frequency_minutes,
                    summary.log_file.display()
                ));
            }
        },
    }
    Ok(())
}
