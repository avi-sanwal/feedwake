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
}

pub struct OpenClawClient {
    wake_url: String,
    token: String,
    mode: String,
    timeout: Duration,
}

impl OpenClawClient {
    pub fn from_config(config: &OpenClawConfig, timeout: Duration) -> Result<Self> {
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
        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();
        let payload = json!({
            "text": event.wake_text(),
            "mode": self.mode,
            "source": event.item.source_name,
            "url": event.item.url,
            "title": event.item.title,
            "matchedRule": event.matched_rule,
            "matchedEntity": event.matched_entity,
            "publishedAt": event.item.published_at.map(|value| value.to_rfc3339()),
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
