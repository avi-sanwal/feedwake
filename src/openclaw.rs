use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use toml_edit::{value, DocumentMut};

use crate::config::default_config_paths;

pub const DEFAULT_HOOK_TOKEN_ENV: &str = "OPENCLAW_HOOK_TOKEN";

const CRON_BLOCK_START: &str = "# feedwake openclaw integration start";
const CRON_BLOCK_END: &str = "# feedwake openclaw integration end";
const DEFAULT_HOOK_PATH: &str = "/hooks";
const DEFAULT_SESSION_KEY: &str = "hook:feedwake";
const DEFAULT_GATEWAY_PORT: u16 = 18789;
const DEFAULT_FEEDWAKE_ACTION_PATH: &str = "feed-wake";
const FEEDWAKE_MESSAGE_TEMPLATE: &str =
    "Review these RSS alerts and explain why they matter:\n\n{{text}}";
const GENERATED_TOKEN_BYTES: usize = 32;
const SYSTEM_LOG_DIR: &str = "/var/log/feedwake";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClawInstallRequest {
    pub openclaw_config_dir: Option<PathBuf>,
    pub feedwake_config_path: Option<PathBuf>,
    pub feedwake_bin: Option<PathBuf>,
    pub frequency_minutes: u8,
    pub hook_token_env: String,
    pub log_file: Option<PathBuf>,
    pub log_max_bytes: u64,
    pub log_rotate_count: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClawInstallOptions {
    pub openclaw_config_dir: PathBuf,
    pub openclaw_config_path: PathBuf,
    pub feedwake_config_path: PathBuf,
    pub feedwake_bin: PathBuf,
    pub frequency_minutes: u8,
    pub hook_token_env: String,
    pub log_file: PathBuf,
    pub log_max_bytes: u64,
    pub log_rotate_count: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClawInstallSummary {
    pub openclaw_config_path: PathBuf,
    pub feedwake_config_path: PathBuf,
    pub feedwake_bin: PathBuf,
    pub frequency_minutes: u8,
    pub log_file: PathBuf,
}

pub fn install_openclaw(request: OpenClawInstallRequest) -> Result<OpenClawInstallSummary> {
    let options = resolve_install_options(request)?;
    let raw_openclaw_config = write_openclaw_config(&options)?;
    write_feedwake_config(&options, &raw_openclaw_config)?;
    install_user_crontab(&options)?;

    Ok(OpenClawInstallSummary {
        openclaw_config_path: options.openclaw_config_path,
        feedwake_config_path: options.feedwake_config_path,
        feedwake_bin: options.feedwake_bin,
        frequency_minutes: options.frequency_minutes,
        log_file: options.log_file,
    })
}

pub fn resolve_install_options(request: OpenClawInstallRequest) -> Result<OpenClawInstallOptions> {
    validate_frequency(request.frequency_minutes)?;
    validate_env_var_name(&request.hook_token_env)?;
    validate_log_rotation(request.log_max_bytes, request.log_rotate_count)?;

    let (openclaw_config_dir, openclaw_config_path) =
        resolve_openclaw_config_location(request.openclaw_config_dir.as_deref())?;
    let feedwake_config_path = match request.feedwake_config_path {
        Some(path) => path,
        None => resolve_feedwake_config_path()?,
    };
    let feedwake_bin = match request.feedwake_bin {
        Some(path) => path,
        None => env::current_exe().context("failed to discover current feedwake executable")?,
    };
    let log_file = match request.log_file {
        Some(path) => path,
        None => resolve_feedwake_log_file()?,
    };

    Ok(OpenClawInstallOptions {
        openclaw_config_dir,
        openclaw_config_path,
        feedwake_config_path,
        feedwake_bin,
        frequency_minutes: request.frequency_minutes,
        hook_token_env: request.hook_token_env,
        log_file,
        log_max_bytes: request.log_max_bytes,
        log_rotate_count: request.log_rotate_count,
    })
}

pub fn patch_openclaw_config(raw_config: &str, hook_token_env: &str) -> Result<Value> {
    validate_env_var_name(hook_token_env)?;

    let mut config: Value = if raw_config.trim().is_empty() {
        json!({})
    } else {
        json5::from_str(raw_config).context("failed to parse OpenClaw JSON5 config")?
    };
    let root = config
        .as_object_mut()
        .context("OpenClaw config root must be an object")?;
    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks
        .as_object_mut()
        .context("OpenClaw hooks config must be an object")?;

    hooks.insert("enabled".to_string(), Value::Bool(true));
    hooks
        .entry("token".to_string())
        .or_insert_with(|| Value::String(format!("${{{}}}", hook_token_env)));
    hooks
        .entry("path".to_string())
        .or_insert_with(|| Value::String(DEFAULT_HOOK_PATH.to_string()));
    hooks
        .entry("defaultSessionKey".to_string())
        .or_insert_with(|| Value::String(DEFAULT_SESSION_KEY.to_string()));
    hooks.insert("allowRequestSessionKey".to_string(), Value::Bool(true));
    hooks
        .entry("allowedSessionKeyPrefixes".to_string())
        .or_insert_with(|| json!(["hook:"]));
    upsert_feedwake_mapping(hooks)?;

    Ok(config)
}

fn upsert_feedwake_mapping(hooks: &mut Map<String, Value>) -> Result<()> {
    let mappings = hooks
        .entry("mappings".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let mappings = mappings
        .as_array_mut()
        .context("OpenClaw hooks.mappings must be an array")?;
    let feedwake_mapping = feedwake_mapping();

    if let Some(existing) = mappings.iter_mut().find(|mapping| {
        mapping.pointer("/match/path").and_then(Value::as_str) == Some(DEFAULT_FEEDWAKE_ACTION_PATH)
    }) {
        let existing = existing
            .as_object_mut()
            .context("OpenClaw feed-wake mapping must be an object")?;
        let feedwake_mapping = feedwake_mapping
            .as_object()
            .context("FeedWake mapping must be an object")?;
        for (key, value) in feedwake_mapping {
            existing.insert(key.clone(), value.clone());
        }
        return Ok(());
    }

    mappings.push(feedwake_mapping);
    Ok(())
}

fn feedwake_mapping() -> Value {
    json!({
        "match": { "path": DEFAULT_FEEDWAKE_ACTION_PATH },
        "action": "agent",
        "name": "FeedWake",
        "wakeMode": "now",
        "messageTemplate": FEEDWAKE_MESSAGE_TEMPLATE,
    })
}

pub fn generate_hook_token() -> Result<String> {
    let mut bytes = [0u8; GENERATED_TOKEN_BYTES];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| anyhow!("failed to generate OpenClaw hook token: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn reconcile_openclaw_env_file(
    existing_env: &str,
    token_env: &str,
    token_value: &str,
) -> Result<String> {
    validate_env_var_name(token_env)?;
    if token_value.is_empty() {
        bail!("OpenClaw hook token value cannot be empty");
    }

    let mut updated = false;
    let mut lines = Vec::new();
    for line in existing_env.lines() {
        let assignment = env_assignment_name(line);
        if line.trim_start().starts_with('#') || assignment.as_deref() != Some(token_env) {
            lines.push(line.to_string());
            continue;
        }
        lines.push(format_env_assignment(token_env, token_value));
        updated = true;
    }

    if !updated {
        lines.push(format_env_assignment(token_env, token_value));
    }

    let mut rendered = lines.join("\n");
    rendered.push('\n');
    Ok(rendered)
}

pub fn render_default_feedwake_config(
    raw_openclaw_config: &str,
    hook_token_env: &str,
) -> Result<String> {
    reconcile_feedwake_config(
        default_feedwake_config_template(),
        raw_openclaw_config,
        hook_token_env,
        None,
    )
}

pub fn reconcile_feedwake_config(
    raw_feedwake_config: &str,
    raw_openclaw_config: &str,
    hook_token_env: &str,
    token_env_file: Option<&Path>,
) -> Result<String> {
    let endpoint = resolve_openclaw_endpoint(raw_openclaw_config, hook_token_env)?;
    let mut document = raw_feedwake_config
        .parse::<DocumentMut>()
        .context("failed to parse FeedWake TOML config")?;

    document["openclaw"]["wake_url"] = value(endpoint.wake_url);
    document["openclaw"]["token_env"] = value(endpoint.token_env);
    if let Some(token_env_file) = token_env_file {
        document["openclaw"]["token_env_file"] =
            value(token_env_file.to_string_lossy().to_string());
    }
    if !document["openclaw"]["mode"].is_value() {
        document["openclaw"]["mode"] = value("now");
    }

    Ok(document.to_string())
}

pub fn render_managed_crontab(existing: &str, options: &OpenClawInstallOptions) -> Result<String> {
    validate_frequency(options.frequency_minutes)?;
    validate_log_rotation(options.log_max_bytes, options.log_rotate_count)?;

    let mut retained = Vec::new();
    let mut in_managed_block = false;
    for line in existing.lines() {
        if line == CRON_BLOCK_START {
            if in_managed_block {
                bail!("existing crontab contains nested FeedWake managed blocks");
            }
            in_managed_block = true;
            continue;
        }
        if line == CRON_BLOCK_END {
            if !in_managed_block {
                bail!("existing crontab contains a FeedWake block end without a start");
            }
            in_managed_block = false;
            continue;
        }
        if !in_managed_block {
            retained.push(line.to_string());
        }
    }

    if in_managed_block {
        bail!("existing crontab contains an unterminated FeedWake managed block");
    }

    while retained.last().map(|line| line.is_empty()).unwrap_or(false) {
        retained.pop();
    }

    let mut rendered = String::new();
    if !retained.is_empty() {
        rendered.push_str(&retained.join("\n"));
        rendered.push('\n');
    }
    rendered.push_str(CRON_BLOCK_START);
    rendered.push('\n');
    rendered.push_str(&cron_entry(options));
    rendered.push('\n');
    rendered.push_str(CRON_BLOCK_END);
    rendered.push('\n');

    Ok(rendered)
}

fn write_openclaw_config(options: &OpenClawInstallOptions) -> Result<String> {
    fs::create_dir_all(&options.openclaw_config_dir).with_context(|| {
        format!(
            "failed to create OpenClaw config directory {}",
            options.openclaw_config_dir.display()
        )
    })?;

    let raw_config = match fs::read_to_string(&options.openclaw_config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}".to_string(),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read OpenClaw config {}",
                    options.openclaw_config_path.display()
                )
            })
        }
    };
    let mut patched = patch_openclaw_config(&raw_config, &options.hook_token_env)?;
    let (token_env, existing_literal_token) =
        normalize_openclaw_hook_token(&mut patched, &options.hook_token_env)?;
    ensure_openclaw_env_token(
        &options.openclaw_config_dir,
        &token_env,
        existing_literal_token.as_deref(),
    )?;
    let rendered = serde_json::to_string_pretty(&patched)
        .context("failed to render patched OpenClaw config")?;
    fs::write(&options.openclaw_config_path, format!("{rendered}\n")).with_context(|| {
        format!(
            "failed to write OpenClaw config {}",
            options.openclaw_config_path.display()
        )
    })?;

    Ok(rendered)
}

fn normalize_openclaw_hook_token(
    config: &mut Value,
    hook_token_env: &str,
) -> Result<(String, Option<String>)> {
    validate_env_var_name(hook_token_env)?;
    let hooks = config
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .context("OpenClaw hooks config must be an object")?;
    let token = hooks
        .get("token")
        .and_then(Value::as_str)
        .context("OpenClaw hooks.token must be a string")?
        .to_string();

    if let Some(token_env) = extract_env_var_reference(&token) {
        return Ok((token_env, None));
    }

    hooks.insert(
        "token".to_string(),
        Value::String(format!("${{{}}}", hook_token_env)),
    );
    Ok((hook_token_env.to_string(), Some(token)))
}

fn ensure_openclaw_env_token(
    openclaw_config_dir: &Path,
    token_env: &str,
    preferred_token: Option<&str>,
) -> Result<()> {
    validate_env_var_name(token_env)?;
    let env_file = openclaw_config_dir.join(".env");
    let existing_env = match fs::read_to_string(&env_file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to read OpenClaw env file {}", env_file.display())
            })
        }
    };

    if env_file_contains_token(&existing_env, token_env) {
        return Ok(());
    }

    let token_value = match preferred_token {
        Some(token) if !token.is_empty() => token.to_string(),
        _ => env::var(token_env)
            .ok()
            .filter(|value| !value.is_empty())
            .map(Ok)
            .unwrap_or_else(generate_hook_token)?,
    };

    let rendered = reconcile_openclaw_env_file(&existing_env, token_env, &token_value)?;
    fs::create_dir_all(openclaw_config_dir).with_context(|| {
        format!(
            "failed to create OpenClaw config directory {}",
            openclaw_config_dir.display()
        )
    })?;
    fs::write(&env_file, rendered)
        .with_context(|| format!("failed to write OpenClaw env file {}", env_file.display()))?;

    Ok(())
}

fn env_file_contains_token(existing_env: &str, token_env: &str) -> bool {
    existing_env
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .any(|line| {
            env_assignment(line)
                .filter(|(name, _)| *name == token_env)
                .map(|(_, value)| !value.trim().is_empty())
                .unwrap_or(false)
        })
}

fn format_env_assignment(name: &str, value: &str) -> String {
    format!("export {name}={}", shell_quote(value))
}

fn env_assignment_name(line: &str) -> Option<String> {
    env_assignment(line).map(|(name, _)| name.to_string())
}

fn env_assignment(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (name, value) = line.split_once('=')?;
    Some((name.trim(), value))
}

fn write_feedwake_config(
    options: &OpenClawInstallOptions,
    raw_openclaw_config: &str,
) -> Result<()> {
    let raw_feedwake_config = match fs::read_to_string(&options.feedwake_config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            default_feedwake_config_template().to_string()
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read FeedWake config {}",
                    options.feedwake_config_path.display()
                )
            })
        }
    };

    let rendered = reconcile_feedwake_config(
        &raw_feedwake_config,
        raw_openclaw_config,
        &options.hook_token_env,
        Some(&options.openclaw_config_dir.join(".env")),
    )?;
    if let Some(parent) = options.feedwake_config_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create FeedWake config directory {}",
                parent.display()
            )
        })?;
    }
    fs::write(&options.feedwake_config_path, rendered).with_context(|| {
        format!(
            "failed to write FeedWake config {}",
            options.feedwake_config_path.display()
        )
    })?;

    Ok(())
}

fn install_user_crontab(options: &OpenClawInstallOptions) -> Result<()> {
    let existing = read_user_crontab()?;
    let rendered = render_managed_crontab(&existing, options)?;
    write_user_crontab(&rendered)
}

fn read_user_crontab() -> Result<String> {
    let output = Command::new("crontab")
        .arg("-l")
        .output()
        .context("failed to read current user crontab")?;
    if output.status.success() {
        return String::from_utf8(output.stdout).context("current crontab is not valid UTF-8");
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("no crontab") {
        return Ok(String::new());
    }

    Err(anyhow!(
        "failed to read current user crontab: {}",
        stderr.trim()
    ))
}

fn write_user_crontab(contents: &str) -> Result<()> {
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to start crontab writer")?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .context("failed to open crontab writer stdin")?;
        stdin
            .write_all(contents.as_bytes())
            .context("failed to write updated crontab")?;
    }

    let status = child.wait().context("failed to wait for crontab writer")?;
    if !status.success() {
        bail!("failed to install updated crontab");
    }

    Ok(())
}

fn resolve_openclaw_config_location(explicit_dir: Option<&Path>) -> Result<(PathBuf, PathBuf)> {
    if let Some(dir) = explicit_dir {
        let dir = dir.to_path_buf();
        let path = dir.join("openclaw.json");
        return Ok((dir, path));
    }

    if let Some(path) = env::var_os("OPENCLAW_CONFIG_PATH") {
        let path = PathBuf::from(path);
        let dir = path
            .parent()
            .context("OPENCLAW_CONFIG_PATH must include a parent directory")?
            .to_path_buf();
        return Ok((dir, path));
    }

    let home = env::var_os("HOME").context("HOME is not set; pass --openclaw-config-dir")?;
    let dir = PathBuf::from(home).join(".openclaw");
    let path = dir.join("openclaw.json");
    Ok((dir, path))
}

fn resolve_feedwake_config_path() -> Result<PathBuf> {
    if let Some(existing_path) = default_config_paths()
        .into_iter()
        .find(|path| path.exists())
    {
        return Ok(existing_path);
    }

    let home = env::var_os("HOME").context("HOME is not set; pass --config")?;
    Ok(PathBuf::from(home).join(".config/feedwake/config.toml"))
}

fn resolve_feedwake_log_file() -> Result<PathBuf> {
    resolve_feedwake_log_file_with_system_dir(Path::new(SYSTEM_LOG_DIR))
}

fn resolve_feedwake_log_file_with_system_dir(system_log_dir: &Path) -> Result<PathBuf> {
    if writable_existing_dir(system_log_dir) {
        return Ok(system_log_dir.join("feedwake.log"));
    }

    if let Some(state_home) = env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(state_home).join("feedwake/feedwake.log"));
    }

    let home = env::var_os("HOME").context("HOME is not set; pass --log-file")?;
    Ok(PathBuf::from(home).join(".local/state/feedwake/feedwake.log"))
}

fn writable_existing_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    let probe = path.join(format!(".feedwake-write-test-{}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenClawEndpoint {
    wake_url: String,
    token_env: String,
}

fn resolve_openclaw_endpoint(
    raw_openclaw_config: &str,
    hook_token_env: &str,
) -> Result<OpenClawEndpoint> {
    validate_env_var_name(hook_token_env)?;
    let openclaw_config = parse_openclaw_config(raw_openclaw_config)?;
    let port = openclaw_config
        .pointer("/gateway/port")
        .and_then(Value::as_u64)
        .map(validate_port)
        .transpose()?
        .unwrap_or(DEFAULT_GATEWAY_PORT);
    let hook_path = openclaw_config
        .pointer("/hooks/path")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_HOOK_PATH);
    let token_env = openclaw_config
        .pointer("/hooks/token")
        .and_then(Value::as_str)
        .and_then(extract_env_var_reference)
        .unwrap_or_else(|| hook_token_env.to_string());

    Ok(OpenClawEndpoint {
        wake_url: format!(
            "http://127.0.0.1:{port}/{}/{}",
            hook_path.trim_matches('/'),
            DEFAULT_FEEDWAKE_ACTION_PATH
        ),
        token_env,
    })
}

fn parse_openclaw_config(raw_config: &str) -> Result<Value> {
    if raw_config.trim().is_empty() {
        Ok(json!({}))
    } else {
        json5::from_str(raw_config).context("failed to parse OpenClaw JSON5 config")
    }
}

fn validate_port(port: u64) -> Result<u16> {
    if (1..=u16::MAX as u64).contains(&port) {
        Ok(port as u16)
    } else {
        bail!("OpenClaw gateway.port must be between 1 and {}", u16::MAX)
    }
}

fn extract_env_var_reference(value: &str) -> Option<String> {
    let name = value.strip_prefix("${")?.strip_suffix('}')?;
    validate_env_var_name(name).ok()?;
    Some(name.to_string())
}

fn default_feedwake_config_template() -> &'static str {
    r#"[openclaw]
wake_url = "http://127.0.0.1:18789/hooks/feed-wake"
token_env = "OPENCLAW_HOOK_TOKEN"
mode = "now"
max_articles_per_wake = 3

[scan]
timeout_seconds = 10
max_items_per_feed = 30
max_response_bytes = 1048576
conditional_get = true

[[watchlist]]
symbol = "RELIANCE"
name = "Reliance Industries Limited"
isin = "INE002A01018"
aliases = ["Reliance Industries", "RIL"]

[[watchlist]]
symbol = "HDFCBANK"
name = "HDFC Bank Limited"
isin = "INE040A01034"
aliases = ["HDFC Bank"]

[[feeds]]
name = "NSE Announcements"
url = "https://nsearchives.nseindia.com/content/RSS/Online_announcements.xml"
source_type = "nse"
filter_profile = "exchange_watchlist"

[[feeds]]
name = "SEBI"
url = "https://www.sebi.gov.in/sebirss.xml"
source_type = "sebi"
filter_profile = "authority_passthrough"

[[feeds]]
name = "RBI Press Releases"
url = "https://rbi.org.in/pressreleases_rss.xml"
source_type = "rbi"
filter_profile = "authority_passthrough"

[[feeds]]
name = "Livemint Markets"
url = "https://www.livemint.com/rss/markets"
source_type = "media"
filter_profile = "media_high_precision"
user_agent = "Mozilla/5.0"

[exchange_filters]
category_allowlist = [
  "Financial Results",
  "Board Meeting",
  "Corporate Action",
  "Insider Trading",
  "Credit Rating",
  "Default",
  "Fund Raising",
  "Acquisition",
  "Litigation",
  "Order"
]

[media_filters]
require_watchlist_match = true
keyword_groups = [
  "results",
  "guidance",
  "merger",
  "acquisition",
  "stake sale",
  "fundraise",
  "default",
  "rating downgrade",
  "rating upgrade",
  "regulatory action",
  "tax demand",
  "large order",
  "management change"
]
exclude_keywords = [
  "sponsored",
  "lifestyle",
  "opinion",
  "explained",
  "personal finance"
]
"#
}

fn cron_entry(options: &OpenClawInstallOptions) -> String {
    escape_cron_percent(&format!(
        "*/{frequency} * * * * {bin} --verbose scan --config {config} --log-file {log_file} --log-max-bytes {log_max_bytes} --log-rotate-count {log_rotate_count}",
        frequency = options.frequency_minutes,
        bin = shell_quote_path(&options.feedwake_bin),
        config = shell_quote_path(&options.feedwake_config_path),
        log_file = shell_quote_path(&options.log_file),
        log_max_bytes = options.log_max_bytes,
        log_rotate_count = options.log_rotate_count,
    ))
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.to_string_lossy())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn escape_cron_percent(value: &str) -> String {
    value.replace('%', r"\%")
}

fn validate_frequency(frequency_minutes: u8) -> Result<()> {
    if (1..=59).contains(&frequency_minutes) {
        Ok(())
    } else {
        bail!("frequency must be between 1 and 59 minutes")
    }
}

fn validate_log_rotation(log_max_bytes: u64, log_rotate_count: u8) -> Result<()> {
    if log_max_bytes == 0 {
        bail!("log max bytes must be greater than 0")
    }
    if log_rotate_count > 30 {
        bail!("log rotate count must be between 0 and 30")
    }
    Ok(())
}

fn validate_env_var_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first = chars
        .next()
        .context("hook token environment variable name cannot be empty")?;
    if !(first == '_' || first.is_ascii_uppercase()) {
        bail!("hook token environment variable name must start with A-Z or _");
    }
    if chars.any(|value| !(value == '_' || value.is_ascii_uppercase() || value.is_ascii_digit())) {
        bail!("hook token environment variable name must contain only A-Z, 0-9, or _");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_feedwake_log_file_with_system_dir;

    #[test]
    fn default_log_file_prefers_writable_system_log_directory() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");

        let log_file = resolve_feedwake_log_file_with_system_dir(temp_dir.path())
            .expect("log path should resolve");

        assert_eq!(log_file, temp_dir.path().join("feedwake.log"));
    }
}
