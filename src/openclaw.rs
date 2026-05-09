use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::load_config;

pub const DEFAULT_HOOK_TOKEN_ENV: &str = "OPENCLAW_HOOK_TOKEN";

const CRON_BLOCK_START: &str = "# feedwake openclaw integration start";
const CRON_BLOCK_END: &str = "# feedwake openclaw integration end";
const DEFAULT_HOOK_PATH: &str = "/hooks";
const DEFAULT_SESSION_KEY: &str = "hook:feedwake";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClawInstallRequest {
    pub openclaw_config_dir: Option<PathBuf>,
    pub feedwake_config_path: Option<PathBuf>,
    pub feedwake_bin: Option<PathBuf>,
    pub frequency_minutes: u8,
    pub hook_token_env: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClawInstallOptions {
    pub openclaw_config_dir: PathBuf,
    pub openclaw_config_path: PathBuf,
    pub feedwake_config_path: PathBuf,
    pub feedwake_bin: PathBuf,
    pub frequency_minutes: u8,
    pub hook_token_env: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClawInstallSummary {
    pub openclaw_config_path: PathBuf,
    pub feedwake_config_path: PathBuf,
    pub feedwake_bin: PathBuf,
    pub frequency_minutes: u8,
}

pub fn install_openclaw(request: OpenClawInstallRequest) -> Result<OpenClawInstallSummary> {
    let options = resolve_install_options(request)?;
    write_openclaw_config(&options)?;
    install_user_crontab(&options)?;

    Ok(OpenClawInstallSummary {
        openclaw_config_path: options.openclaw_config_path,
        feedwake_config_path: options.feedwake_config_path,
        feedwake_bin: options.feedwake_bin,
        frequency_minutes: options.frequency_minutes,
    })
}

pub fn resolve_install_options(request: OpenClawInstallRequest) -> Result<OpenClawInstallOptions> {
    validate_frequency(request.frequency_minutes)?;
    validate_env_var_name(&request.hook_token_env)?;

    let (openclaw_config_dir, openclaw_config_path) =
        resolve_openclaw_config_location(request.openclaw_config_dir.as_deref())?;
    let feedwake_config_path = match request.feedwake_config_path {
        Some(path) => path,
        None => load_config(None)?.1,
    };
    let feedwake_bin = match request.feedwake_bin {
        Some(path) => path,
        None => env::current_exe().context("failed to discover current feedwake executable")?,
    };

    Ok(OpenClawInstallOptions {
        openclaw_config_dir,
        openclaw_config_path,
        feedwake_config_path,
        feedwake_bin,
        frequency_minutes: request.frequency_minutes,
        hook_token_env: request.hook_token_env,
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
    hooks
        .entry("allowRequestSessionKey".to_string())
        .or_insert(Value::Bool(false));
    hooks
        .entry("allowedSessionKeyPrefixes".to_string())
        .or_insert_with(|| json!(["hook:"]));

    Ok(config)
}

pub fn render_managed_crontab(existing: &str, options: &OpenClawInstallOptions) -> Result<String> {
    validate_frequency(options.frequency_minutes)?;

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

fn write_openclaw_config(options: &OpenClawInstallOptions) -> Result<()> {
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
    let patched = patch_openclaw_config(&raw_config, &options.hook_token_env)?;
    let rendered = serde_json::to_string_pretty(&patched)
        .context("failed to render patched OpenClaw config")?;
    fs::write(&options.openclaw_config_path, format!("{rendered}\n")).with_context(|| {
        format!(
            "failed to write OpenClaw config {}",
            options.openclaw_config_path.display()
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

fn cron_entry(options: &OpenClawInstallOptions) -> String {
    let env_file = options.openclaw_config_dir.join(".env");
    let script = format!(
        "if [ -f {env_file} ]; then . {env_file}; fi; exec {bin} scan --config {config}",
        env_file = shell_quote_path(&env_file),
        bin = shell_quote_path(&options.feedwake_bin),
        config = shell_quote_path(&options.feedwake_config_path),
    );
    escape_cron_percent(&format!(
        "*/{} * * * * /bin/sh -c {}",
        options.frequency_minutes,
        shell_quote(&script)
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
