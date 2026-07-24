use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::process::Child;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use rtmp_rs::protocol::message::{ConnectParams, PublishParams};
use rtmp_rs::session::context::StreamContext;
use rtmp_rs::session::SessionContext;
use rtmp_rs::{AuthResult, RtmpHandler, RtmpServer, ServerConfig};

// Embed standalone systemd unit template at compile time
const SYSTEMD_UNIT_TEMPLATE: &str = include_str!("../systemd/rtmp-proxy.service");

#[derive(Parser, Debug)]
#[command(author, version, about = "RTMP Stream Multiplexer powered by rtmp-rs", long_about = None)]
struct CliArgs {
    /// Path to TOML configuration file
    #[arg(short, long, env = "CONFIG_PATH", default_value = "config.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Install systemd unit service for self-managed background execution
    InstallSystemd {
        /// Working directory for systemd service
        #[arg(long, default_value = "/opt/rtmp-proxy")]
        work_dir: PathBuf,

        /// Path to config file for systemd service
        #[arg(long, default_value = "/opt/rtmp-proxy/config.toml")]
        config_path: PathBuf,
    },
}

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
    "0.0.0.0:8080".parse().unwrap()
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

#[derive(Default)]
struct Metrics {
    total_connections: AtomicU64,
    active_connections: AtomicU64,
}

struct ProxyState {
    metrics: Arc<Metrics>,
    notifications: NotificationSettings,
    targets: Vec<TargetConfig>,
    active_relays: Mutex<HashMap<String, Vec<Child>>>,
    listen_port: u16,
    http_client: reqwest::Client,
}

struct ProxyHandler {
    state: Arc<ProxyState>,
}

impl RtmpHandler for ProxyHandler {
    async fn on_connect(&self, ctx: &SessionContext, params: &ConnectParams) -> AuthResult {
        info!(
            session_id = %ctx.session_id,
            client_ip = %ctx.peer_addr,
            app = %params.app,
            "OBS / Client connected to RTMP Ingest"
        );
        self.state.metrics.total_connections.fetch_add(1, Ordering::Relaxed);
        self.state.metrics.active_connections.fetch_add(1, Ordering::Relaxed);
        AuthResult::Accept
    }

    async fn on_publish(&self, ctx: &SessionContext, params: &PublishParams) -> AuthResult {
        let stream_key = params.stream_key.clone();
        info!(
            session_id = %ctx.session_id,
            stream_key = %stream_key,
            "Stream published from client"
        );

        let active_targets: Vec<_> = self.state.targets.iter().filter(|t| t.enabled).cloned().collect();

        // Dispatch going-live webhook notifications in background
        let state = Arc::clone(&self.state);
        let key_clone = stream_key.clone();
        let target_names: Vec<String> = active_targets.iter().map(|t| t.name.clone()).collect();
        tokio::spawn(async move {
            dispatch_notifications(&state, &key_clone, &target_names).await;
        });

        if active_targets.is_empty() {
            info!("No active targets enabled. Ingesting stream locally without forwarding.");
            return AuthResult::Accept;
        }

        let mut relays = self.state.active_relays.lock().await;
        let mut children = Vec::new();
        let source_url = format!("rtmp://127.0.0.1:{}/live/{}", self.state.listen_port, stream_key);

        for target in active_targets {
            info!(name = %target.name, url = %target.url, "Launching stream relay forwarder");

            let child = tokio::process::Command::new("ffmpeg")
                .args([
                    "-loglevel", "warning",
                    "-i", &source_url,
                    "-c", "copy",
                    "-f", "flv",
                    &target.url,
                ])
                .spawn();

            match child {
                Ok(c) => {
                    info!(name = %target.name, "Relay process spawned successfully");
                    children.push(c);
                }
                Err(e) => {
                    error!(name = %target.name, error = %e, "Failed to spawn ffmpeg relay process. Ensure ffmpeg is installed.");
                }
            }
        }

        relays.insert(stream_key, children);
        AuthResult::Accept
    }

    async fn on_unpublish(&self, ctx: &StreamContext) {
        info!(stream_key = %ctx.stream_key, "Stream stopped publishing");

        let mut relays = self.state.active_relays.lock().await;
        if let Some(mut children) = relays.remove(&ctx.stream_key) {
            for mut child in children.drain(..) {
                let _ = child.kill().await;
            }
            info!(stream_key = %ctx.stream_key, "Stopped all active relay forwarders");
        }
    }

    async fn on_disconnect(&self, ctx: &SessionContext) {
        info!(session_id = %ctx.session_id, "Client disconnected");
        self.state.metrics.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn dispatch_notifications(state: &Arc<ProxyState>, stream_key: &str, target_names: &[String]) {
    let msg = &state.notifications.live_message;

    // 1. Dispatch Discord Webhook if configured
    if let Some(ref discord_url) = state.notifications.discord_webhook {
        if !discord_url.trim().is_empty() {
            info!("Sending Discord going-live webhook notification");
            let target_str = if target_names.is_empty() {
                "Ingest-only mode".to_string()
            } else {
                target_names.join(", ")
            };

            let payload = serde_json::json!({
                "content": msg,
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
                        ],
                        "timestamp": chrono_iso_now()
                    }
                ]
            });

            if let Err(e) = state.http_client.post(discord_url).json(&payload).send().await {
                warn!(error = %e, "Failed to send Discord webhook notification");
            }
        }
    }

    // 2. Dispatch Generic Webhook URL if configured
    if let Some(ref webhook_url) = state.notifications.webhook_url {
        if !webhook_url.trim().is_empty() {
            info!(url = %webhook_url, "Sending generic stream.started webhook notification");
            let payload = serde_json::json!({
                "event": "stream.started",
                "stream_key": stream_key,
                "message": msg,
                "targets": target_names,
                "timestamp": chrono_iso_now()
            });

            if let Err(e) = state.http_client.post(webhook_url).json(&payload).send().await {
                warn!(error = %e, "Failed to send generic webhook notification");
            }
        }
    }
}

fn chrono_iso_now() -> String {
    // Generate ISO-8601 timestamp string
    let now = std::time::SystemTime::now();
    let dt: std::time::Duration = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{}", dt.as_secs())
}

fn install_systemd(work_dir: &Path, config_path: &Path) -> Result<()> {
    let current_exe = std::env::current_exe()
        .context("Failed to determine path of current executable")?;

    let unit_content = SYSTEMD_UNIT_TEMPLATE
        .replace("{WORK_DIR}", &work_dir.display().to_string())
        .replace("{EXEC_START}", &current_exe.display().to_string())
        .replace("{CONFIG_PATH}", &config_path.display().to_string());

    let unit_path = Path::new("/etc/systemd/system/rtmp-proxy.service");
    fs::write(unit_path, unit_content)
        .with_context(|| format!("Failed to write systemd unit to {}", unit_path.display()))?;

    println!("Wrote systemd unit file to {}", unit_path.display());

    let status = std::process::Command::new("systemctl")
        .args(["daemon-reload"])
        .status();

    if let Ok(s) = status {
        if s.success() {
            println!("Executed systemctl daemon-reload");
        }
    }

    let enable_status = std::process::Command::new("systemctl")
        .args(["enable", "--now", "rtmp-proxy"])
        .status();

    if let Ok(s) = enable_status {
        if s.success() {
            println!("Successfully enabled and started rtmp-proxy systemd service!");
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rtmp_proxy=info,rtmp_rs=info".into()),
        )
        .init();

    let cli = CliArgs::parse();

    if let Some(Commands::InstallSystemd { work_dir, config_path }) = cli.command {
        return install_systemd(&work_dir, &config_path);
    }

    if !cli.config.exists() {
        bail!(
            "Configuration file '{:?}' does not exist. Create the config file or specify --config <path>",
            cli.config
        );
    }

    info!(path = ?cli.config, "Loading configuration file");
    let config = AppConfig::load_from_file(&cli.config)?;
    config.validate()?;

    let metrics = Arc::new(Metrics::default());
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    info!("Starting RTMP Stream Multiplexer v{}", env!("CARGO_PKG_VERSION"));
    info!("Listening for RTMP stream ingest on {}", config.server.listen);

    for t in &config.targets {
        info!(name = %t.name, enabled = t.enabled, "Target configured");
    }

    let state = Arc::new(ProxyState {
        metrics: Arc::clone(&metrics),
        notifications: config.notifications,
        targets: config.targets,
        active_relays: Mutex::new(HashMap::new()),
        listen_port: config.server.listen.port(),
        http_client,
    });

    // Spawn Health Check HTTP Server on localhost
    let health_addr = config.server.health_listen;
    let health_metrics = Arc::clone(&metrics);
    tokio::spawn(async move {
        if let Err(e) = run_health_server(health_addr, health_metrics).await {
            warn!("Health check server error: {:#}", e);
        }
    });

    // Configure and run RTMP server
    let server_config = ServerConfig::default().bind(config.server.listen);

    let handler = ProxyHandler { state };
    let server = RtmpServer::new(server_config, handler);

    server.run().await.context("RTMP server failed")?;

    Ok(())
}

async fn run_health_server(addr: SocketAddr, metrics: Arc<Metrics>) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind health check listener to {}", addr))?;

    info!("Health check HTTP server listening on {}", addr);

    loop {
        let (mut stream, _) = listener.accept().await?;
        let active = metrics.active_connections.load(Ordering::Relaxed);
        let total = metrics.total_connections.load(Ordering::Relaxed);

        let body = format!(
            "{{\"status\":\"OK\",\"active_connections\":{},\"total_connections\":{}}}\n",
            active, total
        );

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );

        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.flush().await;
    }
}
