use crate::chat::IncomingChatMessage;
use crate::server::state::ProxyState;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

const YOUTUBE_CHAT_MESSAGES_URL: &str = "https://www.googleapis.com/youtube/v3/liveChat/messages";

#[derive(Debug, Clone)]
pub struct YouTubeChatConfig {
    pub api_key: String,
    pub live_chat_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveChatResponse {
    #[serde(default)]
    next_page_token: Option<String>,
    #[serde(default = "default_polling_interval")]
    polling_interval_millis: u64,
    #[serde(default)]
    items: Vec<LiveChatItem>,
}

fn default_polling_interval() -> u64 {
    5000
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveChatItem {
    id: String,
    snippet: LiveChatSnippet,
    author_details: LiveChatAuthor,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveChatSnippet {
    #[serde(default)]
    display_message: String,
    #[serde(default)]
    has_display_content: bool,
    #[serde(default)]
    published_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveChatAuthor {
    display_name: String,
    #[serde(default)]
    profile_image_url: Option<String>,
}

pub async fn run(state: Arc<ProxyState>, config: YouTubeChatConfig) {
    let mut page_token = None;
    let mut retry_delay = Duration::from_secs(2);

    loop {
        match fetch_page(&state, &config, page_token.as_deref()).await {
            Ok(page) => {
                retry_delay = Duration::from_secs(2);
                page_token = page.next_page_token.clone();

                let mut inbox = state.chat_inbox.lock().await;
                for item in page.items {
                    if !item.snippet.has_display_content
                        || item.snippet.display_message.trim().is_empty()
                    {
                        continue;
                    }

                    let message = IncomingChatMessage {
                        source: "youtube".into(),
                        external_id: item.id,
                        author: item.author_details.display_name,
                        text: item.snippet.display_message,
                        avatar_url: item.author_details.profile_image_url,
                        sent_at: item.snippet.published_at,
                    };
                    if let Err(error) = inbox.enqueue(message) {
                        tracing::warn!("Discarding invalid YouTube chat message: {}", error);
                    }
                }
                drop(inbox);

                let interval = page.polling_interval_millis.clamp(1000, 60_000);
                tokio::time::sleep(Duration::from_millis(interval)).await;
            }
            Err(error) => {
                tracing::warn!("YouTube chat ingest failed: {:#}", error);
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(60));
            }
        }
    }
}

async fn fetch_page(
    state: &ProxyState,
    config: &YouTubeChatConfig,
    page_token: Option<&str>,
) -> Result<LiveChatResponse> {
    let mut request = state.http_client.get(YOUTUBE_CHAT_MESSAGES_URL).query(&[
        ("liveChatId", config.live_chat_id.as_str()),
        ("part", "id,snippet,authorDetails"),
        ("maxResults", "200"),
        ("key", config.api_key.as_str()),
    ]);
    if let Some(page_token) = page_token {
        request = request.query(&[("pageToken", page_token)]);
    }

    let response = request
        .send()
        .await
        .context("Failed to call the YouTube live chat API")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!(
            "YouTube live chat API returned {}: {}",
            status,
            body.chars().take(500).collect::<String>()
        );
    }

    response
        .json()
        .await
        .context("Failed to decode the YouTube live chat response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_displayable_youtube_chat_messages() {
        let response: LiveChatResponse = serde_json::from_str(
            r#"{
                "nextPageToken": "next",
                "pollingIntervalMillis": 2500,
                "items": [{
                    "id": "message-id",
                    "snippet": {
                        "displayMessage": "Hello chat",
                        "hasDisplayContent": true,
                        "publishedAt": "2026-07-26T07:30:00Z"
                    },
                    "authorDetails": {
                        "displayName": "Viewer",
                        "profileImageUrl": "https://example.test/avatar.png"
                    }
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(response.next_page_token.as_deref(), Some("next"));
        assert_eq!(response.polling_interval_millis, 2500);
        assert_eq!(response.items[0].id, "message-id");
        assert_eq!(response.items[0].snippet.display_message, "Hello chat");
        assert_eq!(response.items[0].author_details.display_name, "Viewer");
    }
}
