use anyhow::{Context, Result};
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
}

#[derive(Default)]
struct Metrics {
    total_connections: AtomicU64,
    active_connections: AtomicU64,
}

struct ProxyState {
    metrics: Arc<Metrics>,
    targets: Vec<TargetConfig>,
    active_relays: Mutex<HashMap<String, Vec<Child>>>,
    listen_port: u16,
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
            "Stream published from OBS"
        );

        let active_targets: Vec<_> = self.state.targets.iter().filter(|t| t.enabled).collect();

        if !active_targets.is_empty() {
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
        } else {
            info!("No enabled forward targets configured. Ingesting stream without multiplexing.");
        }

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

    let config = if cli.config.exists() {
        info!(path = ?cli.config, "Loading configuration file");
        AppConfig::load_from_file(&cli.config)?
    } else {
        warn!(path = ?cli.config, "Config file not found, using default settings");
        AppConfig::default()
    };

    let metrics = Arc::new(Metrics::default());

    info!("Starting RTMP Stream Multiplexer v{}", env!("CARGO_PKG_VERSION"));
    info!("Listening for OBS on {}", config.server.listen);

    for t in &config.targets {
        info!(name = %t.name, enabled = t.enabled, "Target configured");
    }

    let state = Arc::new(ProxyState {
        metrics: Arc::clone(&metrics),
        targets: config.targets,
        active_relays: Mutex::new(HashMap::new()),
        listen_port: config.server.listen.port(),
    });

    // Spawn Health Check HTTP Server
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
