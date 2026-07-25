use super::state::ProxyState;
use rtmp_rs::protocol::message::{ConnectParams, PublishParams};
use rtmp_rs::session::context::StreamContext;
use rtmp_rs::session::SessionContext;
use rtmp_rs::{AuthResult, RtmpHandler};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::{error, info};

pub struct ProxyHandler {
    pub state: Arc<ProxyState>,
}

impl RtmpHandler for ProxyHandler {
    async fn on_connect(&self, ctx: &SessionContext, params: &ConnectParams) -> AuthResult {
        info!(
            session_id = %ctx.session_id,
            client_ip = %ctx.peer_addr,
            app = %params.app,
            "Client connected to RTMP Ingest"
        );
        self.state
            .metrics
            .total_connections
            .fetch_add(1, Ordering::Relaxed);
        self.state
            .metrics
            .active_connections
            .fetch_add(1, Ordering::Relaxed);
        AuthResult::Accept
    }

    async fn on_publish(&self, ctx: &SessionContext, params: &PublishParams) -> AuthResult {
        let stream_key = params.stream_key.clone();
        info!(
            session_id = %ctx.session_id,
            stream_key = %stream_key,
            "Stream published from client"
        );

        let config = self.state.config.read().await;
        let active_targets: Vec<_> = config
            .targets
            .iter()
            .filter(|t| t.enabled)
            .cloned()
            .collect();
        let target_names: Vec<String> = active_targets.iter().map(|t| t.name.clone()).collect();

        let dispatcher = crate::notifications::NotificationDispatcher::new(
            &config.notifications,
            self.state.http_client.clone(),
        );
        drop(config);

        // Dispatch notifications asynchronously
        let key_clone = stream_key.clone();
        tokio::spawn(async move {
            dispatcher.dispatch(&key_clone, &target_names).await;
        });

        if active_targets.is_empty() {
            info!("No active targets enabled. Ingesting stream locally without forwarding.");
            return AuthResult::Accept;
        }

        let mut relays = self.state.active_relays.lock().await;
        let mut children = Vec::new();
        let source_url = format!(
            "rtmp://127.0.0.1:{}/live/{}",
            self.state.listen_port, stream_key
        );

        for target in active_targets {
            info!(name = %target.name, url = %target.url, "Launching stream relay forwarder");

            let child = tokio::process::Command::new("ffmpeg")
                .args([
                    "-loglevel",
                    "warning",
                    "-i",
                    &source_url,
                    "-c",
                    "copy",
                    "-f",
                    "flv",
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
        self.state
            .metrics
            .active_connections
            .fetch_sub(1, Ordering::Relaxed);
    }
}
