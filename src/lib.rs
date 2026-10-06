//! Self-hosted streamable-HTTP MCP server for YNAB.

mod args;
mod auth;
mod config;
mod money;
mod present;
mod server;
mod validate;
mod ynab;

use std::sync::Arc;

use axum::{middleware, routing::get, Json, Router};
use rmcp::transport::{
    streamable_http_server::session::local::LocalSessionManager, StreamableHttpServerConfig,
    StreamableHttpService,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;

pub use config::{Config, StartupError};

pub fn build_router(config: Config) -> Result<Router, String> {
    let client = ynab::YnabClient::from_config(&config)?;
    let allowed_hosts = config.allowed_hosts.clone();
    let token = config.mcp_auth_token.clone();
    let mcp = StreamableHttpService::new(
        move || Ok(server::YnabServer::new(client.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_allowed_hosts(allowed_hosts),
    );
    Ok(Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .nest_service("/mcp", mcp)
        .layer(middleware::from_fn_with_state(token, auth::require_bearer)))
}

pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::from_env()?;
    init_tracing();
    let port = config.port;
    let default_plan = config.ynab_plan_id.is_some();
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!(port, default_plan, "ynab-mcp listening");
    let app = build_router(config)?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "healthy",
        "service": "ynab-mcp",
    }))
}

async fn root() -> Json<Value> {
    Json(json!({
        "service": "ynab-mcp",
        "mcp": "/mcp",
        "health": "/health",
    }))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutdown signal received");
}
