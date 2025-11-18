use axum::{extract::State, http::StatusCode, response::Json, Router};
use serde_json::Value;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub mod background;
pub mod challenge_migrations;
pub mod challenge_runner;
pub mod compose_hash;
pub mod handlers;
pub mod job_distributor;
pub mod middleware;
pub mod models;
pub mod orm_gateway;
pub mod redis_client;
pub mod routes;
pub mod security;
pub mod services;
pub mod state;

pub use handlers::*;
pub use middleware::*;
pub use routes::*;
pub use state::*;

/// Create the main API router
pub fn create_router(state: AppState) -> Router {
    let router = Router::new()
        .merge(routes::challenges::create_router())
        .merge(routes::jobs::create_router())
        .merge(routes::attestation::create_router())
        .merge(routes::results::create_router())
        .merge(routes::config::create_router())
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
        .merge(routes::mechanisms::create_router())
        .merge(routes::network::create_router())
        .merge(routes::validators::create_router());

    // Apply CORS and tracing to all environments
    router
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .fallback(handle_404)
        .with_state(state)
}

/// Handle 404 Not Found
async fn handle_404() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "Not Found",
            "message": "The requested resource was not found on this server"
        })),
    )
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
    state
        .metrics
        .get_metrics()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
