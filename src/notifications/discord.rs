use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};

pub struct DiscordNotifier {
    webhook_url: String,
    live_message: String,
    http_client: Client,
}

impl DiscordNotifier {
    pub fn new(webhook_url: String, live_message: String, http_client: Client) -> Self {
        Self {
            webhook_url,
            live_message,
            http_client,
        }
    }

    pub async fn notify(&self, stream_key: &str, target_names: &[String]) -> Result<()> {
        if self.webhook_url.trim().is_empty() {
            return Ok(());
        }

        info!("Sending Discord going-live webhook notification");
        let target_str = if target_names.is_empty() {
            "Ingest-only mode".to_string()
        } else {
            target_names.join(", ")
        };

        let payload = serde_json::json!({
            "content": self.live_message,
            "embeds": [
                {
                    "title": "🔴 Stream Started",
                    "description": format!("Stream key `{}` is now live!", stream_key),
                    "color": 15258703,
                    "fields": [
                        {
                            "name": "Broadcasting Targets",
                            "value": target_str,
                            "inline": true
                        }
                    ]
                }
            ]
        });

        if let Err(e) = self
            .http_client
            .post(&self.webhook_url)
            .json(&payload)
            .send()
            .await
        {
            warn!(error = %e, "Failed to send Discord webhook notification");
        }

        Ok(())
    }
}
