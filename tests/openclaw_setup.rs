use feedwake::openclaw::{
    generate_hook_token, patch_openclaw_config, reconcile_feedwake_config,
    reconcile_openclaw_env_file, render_managed_crontab, resolve_install_options,
    OpenClawInstallOptions, OpenClawInstallRequest, DEFAULT_HOOK_TOKEN_ENV,
};
use serde_json::json;
use std::path::PathBuf;

#[test]
fn render_default_feedwake_config_uses_openclaw_port_path_and_token_env() {
    let config = feedwake::openclaw::render_default_feedwake_config(
        r#"
        {
          gateway: { port: 19001 },
          hooks: {
            token: "${OPENCLAW_CUSTOM_HOOK_TOKEN}",
            path: "/ingress",
          },
        }
        "#,
        DEFAULT_HOOK_TOKEN_ENV,
    )
    .expect("default config should render");

    assert!(config.contains("[openclaw]\n"));
    assert!(config.contains("wake_url = \"http://127.0.0.1:19001/ingress/wake\""));
    assert!(config.contains("token_env = \"OPENCLAW_CUSTOM_HOOK_TOKEN\""));
    assert!(config.contains("[scan]\n"));
    assert!(config.contains("[[feeds]]\n"));
}

#[test]
fn render_default_feedwake_config_uses_safe_openclaw_defaults() {
    let config = feedwake::openclaw::render_default_feedwake_config("{}", DEFAULT_HOOK_TOKEN_ENV)
        .expect("default config should render");

    assert!(config.contains("wake_url = \"http://127.0.0.1:18789/hooks/wake\""));
    assert!(config.contains("token_env = \"OPENCLAW_HOOK_TOKEN\""));
}

#[test]
fn generate_hook_token_returns_secure_hex_token() {
    let first = generate_hook_token().expect("token should generate");
    let second = generate_hook_token().expect("token should generate");

    assert_eq!(first.len(), 64);
    assert!(first.chars().all(|value| value.is_ascii_hexdigit()));
    assert_ne!(first, second);
}

#[test]
fn reconcile_openclaw_env_file_adds_or_updates_token_value() {
    let rendered = reconcile_openclaw_env_file(
        "OTHER=value\nOPENCLAW_HOOK_TOKEN=old\n",
        DEFAULT_HOOK_TOKEN_ENV,
        "new-token",
    )
    .expect("env file should reconcile");

    assert!(rendered.contains("OTHER=value"));
    assert!(rendered.contains("OPENCLAW_HOOK_TOKEN='new-token'"));
    assert!(!rendered.contains("OPENCLAW_HOOK_TOKEN=old"));
}

#[test]
fn reconcile_feedwake_config_updates_existing_openclaw_values_only() {
    let existing = r#"
[openclaw]
wake_url = "http://127.0.0.1:18789/hooks/wake"
token_env = "OPENCLAW_HOOK_TOKEN"
mode = "later"

[scan]
timeout_seconds = 42
max_items_per_feed = 7
max_response_bytes = 2048
conditional_get = false

[[feeds]]
name = "Custom Feed"
url = "https://example.com/feed.xml"
source_type = "media"
filter_profile = "media_high_precision"
"#;

    let updated = reconcile_feedwake_config(
        existing,
        r#"
        {
          gateway: { port: 19002 },
          hooks: {
            token: "${OPENCLAW_UPDATED_HOOK_TOKEN}",
            path: "/custom-hooks",
          },
        }
        "#,
        DEFAULT_HOOK_TOKEN_ENV,
    )
    .expect("existing config should update");

    assert!(updated.contains("wake_url = \"http://127.0.0.1:19002/custom-hooks/wake\""));
    assert!(updated.contains("token_env = \"OPENCLAW_UPDATED_HOOK_TOKEN\""));
    assert!(updated.contains("mode = \"later\""));
    assert!(updated.contains("timeout_seconds = 42"));
    assert!(updated.contains("name = \"Custom Feed\""));
}

#[test]
fn patch_openclaw_config_enables_hooks_with_env_token() {
    let patched = patch_openclaw_config(
        r#"
        {
          agents: { defaults: { workspace: "~/.openclaw/workspace" } },
        }
        "#,
        DEFAULT_HOOK_TOKEN_ENV,
    )
    .expect("config should patch");

    assert_eq!(
        patched["hooks"],
        json!({
            "enabled": true,
            "token": "${OPENCLAW_HOOK_TOKEN}",
            "path": "/hooks",
            "defaultSessionKey": "hook:feedwake",
            "allowRequestSessionKey": false,
            "allowedSessionKeyPrefixes": ["hook:"]
        })
    );
    assert_eq!(
        patched["agents"]["defaults"]["workspace"],
        "~/.openclaw/workspace"
    );
}

#[test]
fn patch_openclaw_config_preserves_existing_hook_values() {
    let patched = patch_openclaw_config(
        r#"
        {
          hooks: {
            enabled: false,
            token: "${OPENCLAW_EXISTING_HOOK_TOKEN}",
            path: "/ingress",
            mappings: [{ match: { path: "gmail" }, action: "agent" }],
          },
        }
        "#,
        DEFAULT_HOOK_TOKEN_ENV,
    )
    .expect("config should patch");

    assert_eq!(patched["hooks"]["enabled"], true);
    assert_eq!(patched["hooks"]["token"], "${OPENCLAW_EXISTING_HOOK_TOKEN}");
    assert_eq!(patched["hooks"]["path"], "/ingress");
    assert_eq!(
        patched["hooks"]["mappings"],
        json!([{ "match": { "path": "gmail" }, "action": "agent" }])
    );
}

#[test]
fn render_managed_crontab_adds_feedwake_block() {
    let options = OpenClawInstallOptions {
        openclaw_config_dir: PathBuf::from("/Users/test/.openclaw"),
        openclaw_config_path: PathBuf::from("/Users/test/.openclaw/openclaw.json"),
        feedwake_config_path: PathBuf::from("/Users/test/.config/feedwake/config.toml"),
        feedwake_bin: PathBuf::from("/usr/local/bin/feedwake"),
        frequency_minutes: 5,
        hook_token_env: DEFAULT_HOOK_TOKEN_ENV.to_string(),
    };

    let crontab = render_managed_crontab("MAILTO=test@example.com\n", &options)
        .expect("crontab should render");

    assert!(crontab.contains("MAILTO=test@example.com"));
    assert!(crontab.contains("# feedwake openclaw integration start"));
    assert!(crontab.contains("*/5 * * * * /bin/sh -c "));
    assert!(crontab.contains("/Users/test/.openclaw/.env"));
    assert!(crontab.contains("/usr/local/bin/feedwake"));
    assert!(crontab.contains("scan --config"));
}

#[test]
fn resolve_install_options_uses_explicit_openclaw_config_directory() {
    let options = resolve_install_options(OpenClawInstallRequest {
        openclaw_config_dir: Some(PathBuf::from("/tmp/openclaw")),
        feedwake_config_path: Some(PathBuf::from("/tmp/feedwake.toml")),
        feedwake_bin: Some(PathBuf::from("/tmp/feedwake")),
        frequency_minutes: 10,
        hook_token_env: DEFAULT_HOOK_TOKEN_ENV.to_string(),
    })
    .expect("options should resolve");

    assert_eq!(options.openclaw_config_dir, PathBuf::from("/tmp/openclaw"));
    assert_eq!(
        options.openclaw_config_path,
        PathBuf::from("/tmp/openclaw/openclaw.json")
    );
    assert_eq!(
        options.feedwake_config_path,
        PathBuf::from("/tmp/feedwake.toml")
    );
    assert_eq!(options.feedwake_bin, PathBuf::from("/tmp/feedwake"));
    assert_eq!(options.frequency_minutes, 10);
}

#[test]
fn render_managed_crontab_replaces_existing_feedwake_block() {
    let options = OpenClawInstallOptions {
        openclaw_config_dir: PathBuf::from("/Users/test/.openclaw"),
        openclaw_config_path: PathBuf::from("/Users/test/.openclaw/openclaw.json"),
        feedwake_config_path: PathBuf::from("/Users/test/.config/feedwake/config.toml"),
        feedwake_bin: PathBuf::from("/usr/local/bin/feedwake"),
        frequency_minutes: 15,
        hook_token_env: DEFAULT_HOOK_TOKEN_ENV.to_string(),
    };

    let existing = "\
SHELL=/bin/sh
# feedwake openclaw integration start
*/5 * * * * old command
# feedwake openclaw integration end
0 9 * * * another-command
";

    let crontab = render_managed_crontab(existing, &options).expect("crontab should render");

    assert!(!crontab.contains("old command"));
    assert!(crontab.contains("*/15 * * * * /bin/sh -c "));
    assert!(crontab.contains("SHELL=/bin/sh"));
    assert!(crontab.contains("0 9 * * * another-command"));
}

#[test]
fn render_managed_crontab_rejects_invalid_frequency() {
    let options = OpenClawInstallOptions {
        openclaw_config_dir: PathBuf::from("/Users/test/.openclaw"),
        openclaw_config_path: PathBuf::from("/Users/test/.openclaw/openclaw.json"),
        feedwake_config_path: PathBuf::from("/Users/test/.config/feedwake/config.toml"),
        feedwake_bin: PathBuf::from("/usr/local/bin/feedwake"),
        frequency_minutes: 0,
        hook_token_env: DEFAULT_HOOK_TOKEN_ENV.to_string(),
    };

    assert!(render_managed_crontab("", &options).is_err());
}
