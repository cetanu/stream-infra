use crate::config::{AppConfig, ConfigStore};
use crate::metrics::Metrics;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Child;
use tokio::sync::{Mutex, RwLock};

pub struct ProxyState {
    pub metrics: Arc<Metrics>,
    pub config: Arc<RwLock<AppConfig>>,
    pub http_client: Client,
    pub active_relays: Mutex<HashMap<String, Vec<Child>>>,
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
    ) -> Self {
        Self {
            metrics,
            config: Arc::new(RwLock::new(config)),
            http_client,
            active_relays: Mutex::new(HashMap::new()),
            listen_port,
            config_store,
        }
    }
}
