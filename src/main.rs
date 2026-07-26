mod chat;
mod config;
mod metrics;
mod notifications;
mod server;

mod web;

use anyhow::{Context, Result};
use chat::youtube::{YouTubeChatConfig, YouTubeChatTarget};
use clap::{Parser, Subcommand};
use config::ConfigStore;
use metrics::{run_health_server, Metrics};
use server::{
    run_rtmp_server,
    state::{ChatRuntimeConfig, ProxyState},
};
use std::fs;
use std::num::NonZeroU64;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

// Embed standalone systemd unit template at compile time
const SYSTEMD_UNIT_TEMPLATE: &str = include_str!("../systemd/rtmp-proxy.service");

#[derive(Parser, Debug)]
#[command(author, version, about = "RTMP Stream Multiplexer powered by rtmp-rs", long_about = None)]
struct CliArgs {
    /// Path to the SQLite database, or a legacy TOML file to import once
    #[arg(short, long, env = "CONFIG_PATH", default_value = "config.toml")]
    config: PathBuf,

    /// Bearer token accepted by POST /api/chat/ingest
    #[arg(long, env = "CHAT_INGEST_TOKEN", hide_env_values = true)]
    chat_ingest_token: Option<String>,

    /// Maximum number of displayed and waiting chat messages retained in SQLite
    #[arg(long, env = "CHAT_QUEUE_CAPACITY", default_value = "500")]
    chat_queue_capacity: NonZeroUsize,

    /// Secret used to verify Twitch EventSub webhook signatures
    #[arg(long, env = "TWITCH_EVENTSUB_SECRET", hide_env_values = true)]
    twitch_eventsub_secret: Option<String>,

    /// Google API key used to read YouTube live chat
    #[arg(long, env = "YOUTUBE_API_KEY", hide_env_values = true)]
    youtube_api_key: Option<String>,

    /// YouTube liveChatId to ingest
    #[arg(
        long,
        env = "YOUTUBE_LIVE_CHAT_ID",
        conflicts_with_all = ["youtube_video_id", "youtube_channel_id"]
    )]
    youtube_live_chat_id: Option<String>,

    /// YouTube videoId whose active live chat should be discovered
    #[arg(long, env = "YOUTUBE_VIDEO_ID", conflicts_with = "youtube_channel_id")]
    youtube_video_id: Option<String>,

    /// YouTube channelId whose active stream and live chat should be discovered
    #[arg(long, env = "YOUTUBE_CHANNEL_ID")]
    youtube_channel_id: Option<String>,

    /// Minimum interval between YouTube chat API polls
    #[arg(long, env = "YOUTUBE_MIN_POLL_INTERVAL_SECS", default_value = "5")]
    youtube_min_poll_interval_secs: NonZeroU64,

    /// Back off YouTube polling when the chat is idle
    #[arg(
        long,
        env = "YOUTUBE_ADAPTIVE_POLLING",
        default_value = "true",
        action = clap::ArgAction::Set
    )]
    youtube_adaptive_polling: bool,

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

fn install_systemd(work_dir: &Path, config_path: &Path) -> Result<()> {
    let current_exe =
        std::env::current_exe().context("Failed to determine path of current executable")?;

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

    if let Some(Commands::InstallSystemd {
        work_dir,
        config_path,
    }) = cli.command
    {
        return install_systemd(&work_dir, &config_path);
    }

    info!(path = ?cli.config, "Loading configuration");
    let (config_store, config) = ConfigStore::open(&cli.config)?;
    config.validate()?;
    info!(path = %config_store.path().display(), "Using SQLite configuration database");

    let metrics = Arc::new(Metrics::default());
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    info!(
        "Starting RTMP Stream Multiplexer v{}",
        env!("CARGO_PKG_VERSION")
    );
    info!(
        "Listening for RTMP stream ingest on {}",
        config.server.listen
    );

    for t in &config.targets {
        info!(name = %t.name, enabled = t.enabled, "Target configured");
    }

    let web_addr = config.server.api_listen;
    let listen_port = config.server.listen.port();
    let state = Arc::new(ProxyState::new(
        Arc::clone(&metrics),
        config,
        http_client,
        listen_port,
        config_store,
        ChatRuntimeConfig {
            ingest_token: cli.chat_ingest_token,
            queue_capacity: cli.chat_queue_capacity.get(),
            twitch_eventsub_secret: cli.twitch_eventsub_secret,
        },
    )?);

    let youtube_target = cli
        .youtube_live_chat_id
        .filter(|value| !value.trim().is_empty())
        .map(YouTubeChatTarget::LiveChat)
        .or_else(|| {
            cli.youtube_video_id
                .filter(|value| !value.trim().is_empty())
                .map(YouTubeChatTarget::Video)
        })
        .or_else(|| {
            cli.youtube_channel_id
                .filter(|value| !value.trim().is_empty())
                .map(YouTubeChatTarget::Channel)
        });
    match (
        cli.youtube_api_key
            .filter(|api_key| !api_key.trim().is_empty()),
        youtube_target,
    ) {
        (Some(api_key), Some(target)) => {
            let youtube_state = Arc::clone(&state);
            tokio::spawn(chat::youtube::run(
                youtube_state,
                YouTubeChatConfig {
                    api_key,
                    target,
                    min_poll_interval: std::time::Duration::from_secs(
                        cli.youtube_min_poll_interval_secs.get(),
                    ),
                    adaptive_polling: cli.youtube_adaptive_polling,
                },
            ));
            info!("YouTube live chat ingest enabled");
        }
        (None, None) => {}
        _ => warn!(
            "YouTube chat ingest requires YOUTUBE_API_KEY and one of YOUTUBE_LIVE_CHAT_ID, \
             YOUTUBE_VIDEO_ID, or YOUTUBE_CHANNEL_ID; adapter disabled"
        ),
    }

    // Spawn Web Server
    let web_state = Arc::clone(&state);
    tokio::spawn(async move {
        if let Err(e) = crate::web::run_web_server(web_state, web_addr).await {
            warn!("Web interface server error: {:#}", e);
        }
    });

    // Spawn Health Check HTTP Server on localhost
    let health_addr = state.config.read().await.server.health_listen;
    let health_metrics = Arc::clone(&metrics);
    tokio::spawn(async move {
        if let Err(e) = run_health_server(health_addr, health_metrics).await {
            warn!("Health check server error: {:#}", e);
        }
    });

    // Run RTMP Server
    let rtmp_listen = state.config.read().await.server.listen;
    run_rtmp_server(rtmp_listen, state).await
}
