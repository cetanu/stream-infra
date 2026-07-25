pub mod discord;
pub mod generic;

use crate::config::NotificationSettings;
use discord::DiscordNotifier;
use generic::GenericWebhookNotifier;
use reqwest::Client;
use std::sync::Arc;

pub struct NotificationDispatcher {
    discord: Option<DiscordNotifier>,
    generic: Option<GenericWebhookNotifier>,
}

impl NotificationDispatcher {
    pub fn new(settings: &NotificationSettings, http_client: Client) -> Arc<Self> {
        let discord = settings.discord_webhook.as_ref().map(|url| {
            DiscordNotifier::new(
                url.clone(),
                settings.live_message.clone(),
                http_client.clone(),
            )
        });

        let generic = settings.webhook_url.as_ref().map(|url| {
            GenericWebhookNotifier::new(url.clone(), settings.live_message.clone(), http_client)
        });

        Arc::new(Self { discord, generic })
    }

    pub async fn dispatch(&self, stream_key: &str, target_names: &[String]) {
        if let Some(ref d) = self.discord {
            let _ = d.notify(stream_key, target_names).await;
        }

        if let Some(ref g) = self.generic {
            let _ = g.notify(stream_key, target_names).await;
        }
    }
}
