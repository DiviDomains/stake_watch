pub mod api;
pub mod auth;

use axum::Router;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use crate::config::{AppConfig, Secrets};
use crate::db::DbPool;
use crate::rpc::RpcClient;

/// Shared state for the web application, independent of the Telegram bot state.
pub struct WebAppState {
    pub db: DbPool,
    pub rpc: Arc<dyn RpcClient>,
    pub config: Arc<AppConfig>,
    pub secrets: Arc<Secrets>,
    pub explorer_url: String,
    pub block_tx: Option<tokio::sync::broadcast::Sender<String>>,
}

/// Build the axum Router with all API routes and static file serving.
pub fn router(state: Arc<WebAppState>) -> Router {
    let api_routes = api::router(state.clone());

    Router::new()
        .nest("/api", api_routes)
        .fallback_service(ServeDir::new("static").append_index_html_on_directories(true))
        .layer(CorsLayer::permissive())
}
