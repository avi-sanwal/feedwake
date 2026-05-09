use crate::feed::FeedItem;
use anyhow::{anyhow, Context, Result};
use serde_json::json;
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
        let token = std::env::var(&config.token_env)
            .with_context(|| format!("{} is not set", config.token_env))?;
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
