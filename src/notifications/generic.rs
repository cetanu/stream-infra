use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};

pub struct GenericWebhookNotifier {
    webhook_url: String,
    live_message: String,
    http_client: Client,
}

impl GenericWebhookNotifier {
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

        info!(url = %self.webhook_url, "Sending generic stream.started webhook notification");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let payload = serde_json::json!({
            "event": "stream.started",
            "stream_key": stream_key,
            "message": self.live_message,
            "targets": target_names,
            "timestamp": now
        });

        if let Err(e) = self
            .http_client
            .post(&self.webhook_url)
            .json(&payload)
            .send()
            .await
        {
            warn!(error = %e, "Failed to send generic webhook notification");
        }

        Ok(())
    }
}
