use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post, put},
    Router,
};
use serde_json::Value;
use std::collections::HashMap;
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
    use crate::middleware::security::*;

    // Check if production mode
    let is_production =
        std::env::var("ENVIRONMENT_MODE").unwrap_or_else(|_| "dev".to_string()) == "prod";

    let mut router = Router::new()
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

    // Apply security middleware ONLY in production
    // In dev mode, NO authentication middleware is applied
    if is_production {
        use std::sync::Arc;
        use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};

        let rate_limit_config = Arc::new(
            GovernorConfigBuilder::default()
                .per_second(10)
                .burst_size(20)
                .finish()
                .expect("Failed to create rate limiter config"),
        );
        let rate_limit = GovernorLayer {
            config: rate_limit_config,
        };

        router = router
            .layer(axum::middleware::from_fn(security_headers_middleware))
            .layer(axum::middleware::from_fn(request_validation_middleware))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                jwt_auth_middleware,
            ))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                attestation_middleware,
            ))
            .layer(axum::middleware::from_fn(ip_whitelist_middleware))
            .layer(rate_limit);
    } else {
        // In dev mode, log that no auth middleware is applied
        tracing::info!("🔓 Development mode: No authentication middleware applied");
    }

    // Apply CORS and tracing to all environments
    router
        .layer(if is_production {
            cors_layer()
        } else {
            CorsLayer::permissive()
        })
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
    state
        .metrics
        .get_metrics()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
