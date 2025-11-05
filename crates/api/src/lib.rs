use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post, put, delete},
    Router,
};
use serde_json::Value;
use std::collections::HashMap;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod state;
pub mod security;
pub mod compose_hash;
pub mod background;
pub mod challenge_migrations;
pub mod challenge_runner;
pub mod orm_gateway;
pub mod models;
pub mod job_distributor;

pub use handlers::*;
pub use middleware::*;
pub use routes::*;
pub use state::*;

/// Create the main API router
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .merge(routes::challenges::create_router())
        .merge(routes::jobs::create_router())
        .merge(routes::attestation::create_router())
        .merge(routes::results::create_router())
        // NOTE: Config routes for subnet config/backups are disabled - functionality not implemented in storage backend
        // .merge(routes::config::create_router())
        .merge(routes::emissions::create_router())
        .merge(routes::health::create_router())
        // NOTE: Pools and nodes routes are disabled - functionality not implemented in storage backend
        // .merge(routes::pools::create_router())
        // .merge(routes::nodes::create_router())
        .merge(routes::ui::create_router())
        .merge(routes::artifacts::create_router())
        .merge(routes::websocket::create_router())
        .merge(routes::challenge_credentials::create_router())
        .merge(routes::orm::create_router())
        .merge(routes::metagraph::create_router())
        .merge(routes::challenge_proxy::create_router())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Health check endpoint
pub async fn health_check() -> Json<Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now(),
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Metrics endpoint
pub async fn metrics(State(state): State<AppState>) -> Result<String, StatusCode> {
    state.metrics.get_metrics().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}


