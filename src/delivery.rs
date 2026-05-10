use crate::feed::FeedItem;
use anyhow::{anyhow, Context, Result};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::config::OpenClawConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeEvent {
    pub item: FeedItem,
    pub matched_rule: String,
    pub matched_entity: Option<String>,
}

impl WakeEvent {
    pub fn wake_text(&self) -> String {
        format!(
            "Feed alert: {} published \"{}\" {}",
            self.item.source_name, self.item.title, self.item.url
        )
    }

    fn article_payload(&self) -> serde_json::Value {
        json!({
            "source": self.item.source_name,
            "sourceUrl": self.item.source_url,
            "title": self.item.title,
            "url": self.item.url,
            "description": self.item.description,
            "matchedRule": self.matched_rule,
            "matchedEntity": self.matched_entity,
            "publishedAt": self.item.published_at.as_ref().map(|value| value.to_rfc3339()),
        })
    }
}

pub struct OpenClawClient {
    wake_url: String,
    token: String,
    mode: String,
    timeout: Duration,
}

impl OpenClawClient {
    pub fn from_config(config: &OpenClawConfig, timeout: Duration) -> Result<Self> {
        if config.max_articles_per_wake == 0 {
            return Err(anyhow!(
                "openclaw.max_articles_per_wake must be greater than 0"
            ));
        }
        let token = resolve_hook_token(config)?;
        Ok(Self {
            wake_url: config.wake_url.clone(),
            token,
            mode: config.mode.clone(),
            timeout,
        })
    }

    pub fn post(&self, event: &WakeEvent) -> Result<()> {
        self.post_batch(std::slice::from_ref(event))
    }

    pub fn post_batch(&self, events: &[WakeEvent]) -> Result<()> {
        if events.is_empty() {
            return Err(anyhow!(
                "OpenClaw wake delivery requires at least one event"
            ));
        }

        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();
        let articles: Vec<_> = events.iter().map(WakeEvent::article_payload).collect();
        let payload = json!({
            "text": batch_wake_text(events),
            "mode": self.mode,
            "articleCount": events.len(),
            "articles": articles,
        });

        let response = agent
            .post(&self.wake_url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", "application/json")
            .send_json(payload);

        match response {
            Ok(response) if response.status() >= 200 && response.status() < 300 => Ok(()),
            Ok(response) => Err(anyhow!(
                "OpenClaw wake delivery failed with status {}",
                response.status()
            )),
            Err(error) => Err(anyhow!("OpenClaw wake delivery failed: {}", error)),
        }
    }
}

fn batch_wake_text(events: &[WakeEvent]) -> String {
    let mut text = format!("FeedWake matched {} RSS article(s).", events.len());
    for (index, event) in events.iter().enumerate() {
        text.push_str(&format!(
            "\n\n{}. {}\nSource: {}\nURL: {}\nSource feed: {}\nMatched rule: {}",
            index + 1,
            event.item.title,
            event.item.source_name,
            event.item.url,
            event.item.source_url,
            event.matched_rule
        ));
        if let Some(entity) = &event.matched_entity {
            text.push_str(&format!("\nMatched entity: {entity}"));
        }
        if let Some(published_at) = event.item.published_at.as_ref() {
            text.push_str(&format!("\nPublished: {}", published_at.to_rfc3339()));
        }
        if let Some(description) = &event.item.description {
            if !description.trim().is_empty() {
                text.push_str(&format!("\nDescription: {}", description.trim()));
            }
        }
    }
    text
}

fn resolve_hook_token(config: &OpenClawConfig) -> Result<String> {
    if let Ok(token) = std::env::var(&config.token_env) {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    if let Some(token_env_file) = &config.token_env_file {
        let token = read_token_from_env_file(Path::new(token_env_file), &config.token_env)?;
        if !token.is_empty() {
            return Ok(token);
        }
    }

    Err(anyhow!(
        "{} is not set and token_env_file did not provide it",
        config.token_env
    ))
}

fn read_token_from_env_file(path: &Path, token_env: &str) -> Result<String> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read OpenClaw token env file {}", path.display()))?;
    for line in contents.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let Some((name, value)) = env_assignment(line) else {
            continue;
        };
        if name == token_env {
            return parse_env_value(value)
                .with_context(|| format!("failed to parse {token_env} in {}", path.display()));
        }
    }

    Err(anyhow!(
        "{} was not found in OpenClaw token env file {}",
        token_env,
        path.display()
    ))
}

fn env_assignment(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (name, value) = line.split_once('=')?;
    Some((name.trim(), value.trim()))
}

fn parse_env_value(value: &str) -> Result<String> {
    let value = value.trim();
    if let Some(value) = value.strip_prefix('\'') {
        let value = value
            .strip_suffix('\'')
            .context("single-quoted env value is missing closing quote")?;
        return Ok(value.replace("'\\''", "'"));
    }
    if let Some(value) = value.strip_prefix('"') {
        let value = value
            .strip_suffix('"')
            .context("double-quoted env value is missing closing quote")?;
        return Ok(value.replace("\\\"", "\""));
    }
    Ok(value.to_string())
}
