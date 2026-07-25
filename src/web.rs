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
