use feedwake::openclaw::{
    patch_openclaw_config, render_managed_crontab, resolve_install_options, OpenClawInstallOptions,
    OpenClawInstallRequest, DEFAULT_HOOK_TOKEN_ENV,
};
use serde_json::json;
use std::path::PathBuf;

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
