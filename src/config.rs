use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub openclaw: OpenClawConfig,
    pub scan: ScanConfig,
    #[serde(default)]
    pub watchlist: Vec<WatchlistEntry>,
    pub feeds: Vec<FeedConfig>,
    #[serde(default)]
    pub exchange_filters: ExchangeFilters,
    #[serde(default)]
    pub media_filters: MediaFilters,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenClawConfig {
    pub wake_url: String,
    pub token_env: String,
    pub token_env_file: Option<String>,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_max_articles_per_wake")]
    pub max_articles_per_wake: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScanConfig {
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_max_items_per_feed")]
    pub max_items_per_feed: usize,
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: usize,
    #[serde(default = "default_conditional_get")]
    pub conditional_get: bool,
    pub state_db: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchlistEntry {
    pub symbol: String,
    pub name: String,
    pub isin: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeedConfig {
    pub name: String,
    pub url: String,
    pub source_type: SourceType,
    pub filter_profile: FilterProfile,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Nse,
    Bse,
    Sebi,
    Rbi,
    Media,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterProfile {
    ExchangeWatchlist,
    AuthorityPassthrough,
    MediaHighPrecision,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExchangeFilters {
    #[serde(default)]
    pub category_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MediaFilters {
    #[serde(default = "default_require_watchlist_match")]
    pub require_watchlist_match: bool,
    #[serde(default)]
    pub keyword_groups: Vec<String>,
    #[serde(default)]
    pub exclude_keywords: Vec<String>,
}

impl Default for MediaFilters {
    fn default() -> Self {
        Self {
            require_watchlist_match: true,
            keyword_groups: Vec::new(),
            exclude_keywords: Vec::new(),
        }
    }
}

fn default_mode() -> String {
    "now".to_string()
}

fn default_max_articles_per_wake() -> usize {
    3
}

fn default_timeout_seconds() -> u64 {
    10
}

fn default_max_items_per_feed() -> usize {
    30
}

fn default_max_response_bytes() -> usize {
    1_048_576
}

fn default_conditional_get() -> bool {
    true
}

fn default_require_watchlist_match() -> bool {
    true
}

pub fn load_config(explicit_path: Option<&Path>) -> Result<(Config, PathBuf)> {
    let path = match explicit_path {
        Some(path) => path.to_path_buf(),
        None => default_config_paths()
            .into_iter()
            .find(|path| path.exists())
            .context("no config file found; pass --config or create /etc/feedwake.toml")?,
    };

    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    Ok((config, path))
}

pub fn default_config_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("/etc/feedwake.toml")];
    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        paths.push(home.join(".config/feedwake/config.toml"));
        paths.push(home.join(".feedwake.toml"));
    }
    paths
}

pub fn default_state_db_path() -> PathBuf {
    let system_path = PathBuf::from("/var/lib/feedwake/feedwake.db");
    if system_path
        .parent()
        .map(|parent| parent.exists() && is_writable_dir(parent))
        .unwrap_or(false)
    {
        return system_path;
    }

    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/feedwake/feedwake.db");
    }

    PathBuf::from("feedwake.db")
}

fn is_writable_dir(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| !metadata.permissions().readonly())
        .unwrap_or(false)
}
