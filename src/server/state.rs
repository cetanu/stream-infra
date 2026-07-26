use crate::chat::ChatInbox;
use crate::config::{AppConfig, ConfigStore};
use crate::metrics::Metrics;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Child;
use tokio::sync::{Mutex, RwLock};

pub struct ChatRuntimeConfig {
    pub ingest_token: Option<String>,
    pub queue_capacity: usize,
    pub twitch_eventsub_secret: Option<String>,
}

pub struct ProxyState {
    pub metrics: Arc<Metrics>,
    pub config: Arc<RwLock<AppConfig>>,
    pub http_client: Client,
    pub active_relays: Mutex<HashMap<String, Vec<Child>>>,
    pub chat_inbox: Mutex<ChatInbox>,
    pub chat_ingest_token: Option<String>,
    pub twitch_eventsub_secret: Option<String>,
    pub listen_port: u16,
    pub config_store: ConfigStore,
}

impl ProxyState {
    pub fn new(
        metrics: Arc<Metrics>,
        config: AppConfig,
        http_client: Client,
        listen_port: u16,
        config_store: ConfigStore,
        chat: ChatRuntimeConfig,
    ) -> Self {
        Self {
            metrics,
            config: Arc::new(RwLock::new(config)),
            http_client,
            active_relays: Mutex::new(HashMap::new()),
            chat_inbox: Mutex::new(ChatInbox::new(chat.queue_capacity)),
            chat_ingest_token: chat.ingest_token.filter(|token| !token.trim().is_empty()),
            twitch_eventsub_secret: chat
                .twitch_eventsub_secret
                .filter(|secret| !secret.trim().is_empty()),
            listen_port,
            config_store,
        }
    }
}
