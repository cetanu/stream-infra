use crate::config::TargetConfig;
use crate::metrics::Metrics;
use crate::notifications::NotificationDispatcher;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Child;
use tokio::sync::Mutex;

pub struct ProxyState {
    pub metrics: Arc<Metrics>,
    pub dispatcher: Arc<NotificationDispatcher>,
    pub targets: Vec<TargetConfig>,
    pub active_relays: Mutex<HashMap<String, Vec<Child>>>,
    pub listen_port: u16,
}

impl ProxyState {
    pub fn new(
        metrics: Arc<Metrics>,
        dispatcher: Arc<NotificationDispatcher>,
        targets: Vec<TargetConfig>,
        listen_port: u16,
    ) -> Self {
        Self {
            metrics,
            dispatcher,
            targets,
            active_relays: Mutex::new(HashMap::new()),
            listen_port,
        }
    }
}
