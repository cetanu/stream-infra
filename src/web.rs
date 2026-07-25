use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

use crate::config::AppConfig;
use crate::server::state::ProxyState;

pub async fn run_web_server(
    state: Arc<ProxyState>,
    addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(serve_index))
        .route("/api/config", get(get_config))
        .route("/api/config", post(update_config))
        .route("/api/test-webhooks", post(test_webhooks))
        .route("/api/test-stream", post(test_stream))
        .route("/api/metrics", get(get_metrics))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(
        "Web interface listening on http://{}",
        listener.local_addr()?
    );
    axum::serve(listener, app).await?;
    Ok(())
}

async fn serve_index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn get_config(State(state): State<Arc<ProxyState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    Json(config.clone())
}

async fn update_config(
    State(state): State<Arc<ProxyState>>,
    Json(new_config): Json<AppConfig>,
) -> impl IntoResponse {
    if let Err(e) = new_config.validate() {
        return (
            StatusCode::BAD_REQUEST,
            format!("Invalid configuration: {}", e),
        );
    }

    let mut config_write = state.config.write().await;

    // Attempt to save to file
    if let Err(e) = new_config.save_to_file(&state.config_path) {
        tracing::error!("Failed to save configuration to file: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save configuration: {}", e),
        );
    }

    *config_write = new_config;

    (
        StatusCode::OK,
        "Configuration updated successfully".to_string(),
    )
}

async fn test_webhooks(State(state): State<Arc<ProxyState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    let active_targets: Vec<_> = config.targets.iter().filter(|t| t.enabled).cloned().collect();
    
    let dispatcher = crate::notifications::NotificationDispatcher::new(&config.notifications, state.http_client.clone());
    drop(config);

    dispatcher.dispatch("test_stream_webhook_123", &active_targets).await;

    (StatusCode::OK, "Webhooks test dispatched")
}

async fn test_stream(State(state): State<Arc<ProxyState>>) -> impl IntoResponse {
    let listen_port = state.listen_port;
    let url = format!("rtmp://127.0.0.1:{}/live/test_stream", listen_port);
    
    tokio::spawn(async move {
        tracing::info!("Starting 15s test stream via ffmpeg to local ingest...");
        let _ = tokio::process::Command::new("ffmpeg")
            .args([
                "-re",
                "-f", "lavfi", "-i", "testsrc=duration=15:size=1280x720:rate=30",
                "-f", "lavfi", "-i", "sine=frequency=1000:duration=15",
                "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
                "-c:a", "aac", "-b:a", "128k",
                "-f", "flv",
                &url,
            ])
            .output()
            .await;
        tracing::info!("Test stream finished.");
    });

    (StatusCode::OK, "Test stream initiated (15s)")
}

#[derive(serde::Serialize)]
struct WebMetrics {
    active_connections: u64,
    total_connections: u64,
    active_streams: usize,
    active_relays: usize,
}

async fn get_metrics(State(state): State<Arc<ProxyState>>) -> impl IntoResponse {
    let active_connections = state.metrics.active_connections.load(std::sync::atomic::Ordering::Relaxed);
    let total_connections = state.metrics.total_connections.load(std::sync::atomic::Ordering::Relaxed);
    
    let relays_guard = state.active_relays.lock().await;
    let active_streams = relays_guard.len();
    let active_relays = relays_guard.values().map(|v| v.len()).sum();
    drop(relays_guard);

    Json(WebMetrics {
        active_connections,
        total_connections,
        active_streams,
        active_relays,
    })
}
