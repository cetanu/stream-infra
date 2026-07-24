use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerSettings {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,

    #[serde(default = "default_health_listen")]
    pub health_listen: SocketAddr,
}

fn default_listen() -> SocketAddr {
    "0.0.0.0:1935".parse().unwrap()
}

fn default_health_listen() -> SocketAddr {
    "127.0.0.1:8080".parse().unwrap()
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            health_listen: default_health_listen(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct NotificationSettings {
    pub discord_webhook: Option<String>,
    #[serde(default = "default_live_message")]
    pub live_message: String,
    pub webhook_url: Option<String>,
}

fn default_live_message() -> String {
    "Stream is LIVE".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TargetConfig {
    pub name: String,
    pub url: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerSettings,

    #[serde(default)]
    pub notifications: NotificationSettings,

    #[serde(default)]
    pub targets: Vec<TargetConfig>,
}

impl AppConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read configuration file: {:?}", path.as_ref()))?;
        let config: AppConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse TOML configuration from {:?}", path.as_ref()))?;
        Ok(config)
    }

    /// Validate enabled target URLs (allows 0 enabled targets for ingest-only mode)
    pub fn validate(&self) -> Result<()> {
        for target in &self.targets {
            if target.enabled {
                let url = target.url.trim();
                if url.is_empty() {
                    bail!("Target '{}' has an empty RTMP URL.", target.name);
                }
                if !url.starts_with("rtmp://") && !url.starts_with("rtmps://") {
                    bail!(
                        "Target '{}' has invalid URL '{}'. Must start with rtmp:// or rtmps://",
                        target.name,
                        url
                    );
                }
            }
        }
        Ok(())
    }
}
