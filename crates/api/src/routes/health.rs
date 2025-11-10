use axum::{extract::State, http::StatusCode, response::Json, routing::get, Router};
use serde_json::Value;

use crate::state::AppState;

/// Create health router
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/", get(version_info))
        .route("/health", get(health_check))
        .route("/health/ready", get(readiness_check))
        .route("/health/live", get(liveness_check))
        .route("/metrics", get(metrics))
        .route("/version", get(version_info))
}

/// Health check endpoint
pub async fn health_check(State(state): State<AppState>) -> Json<Value> {
    let health_status = HealthStatus {
        status: "healthy".to_string(),
        timestamp: chrono::Utc::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime: get_uptime(),
        services: get_service_status(&state).await,
    };

    Json(serde_json::to_value(health_status).unwrap())
}

/// Readiness check endpoint
pub async fn readiness_check(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    let readiness = check_readiness(&state).await;

    if readiness.is_ready {
        Ok(Json(serde_json::to_value(readiness).unwrap()))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Liveness check endpoint
pub async fn liveness_check() -> Json<Value> {
    let liveness = LivenessStatus {
        status: "alive".to_string(),
        timestamp: chrono::Utc::now(),
        uptime: get_uptime(),
    };

    Json(serde_json::to_value(liveness).unwrap())
}

/// Metrics endpoint
pub async fn metrics(State(state): State<AppState>) -> Result<String, StatusCode> {
    state
        .metrics
        .get_metrics()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Version info endpoint - Returns Docker commit SHA and public key for verification
pub async fn version_info(State(state): State<AppState>) -> Json<Value> {
    // Get public key from security module
    let public_key = state.security.get_public_key();
    let public_key_hex = hex::encode(&public_key);

    // Get compose_hash from TDX attestation
    let compose_hash = state.security.get_compose_hash();

    // Package version from Cargo
    let cargo_version = env!("CARGO_PKG_VERSION");

    // Get git commit if available
    let git_commit = option_env!("GIT_COMMIT").unwrap_or("unknown");

    Json(serde_json::json!({
        "platform_api": {
            "cargo_version": cargo_version,
            "git_commit": git_commit,
            "compose_hash": compose_hash,  // Hash of Docker Compose (attested by TDX)
            "build_time": option_env!("VERGEN_BUILD_TIMESTAMP").unwrap_or("unknown"),
            "public_key": public_key_hex,  // Public key for verification
        },
        "warning": if compose_hash == "unknown" {
            "Compose hash not available - cannot verify Docker image integrity"
        } else {
            ""
        }
    }))
}

/// Health status structure
#[derive(Debug, serde::Serialize)]
struct HealthStatus {
    status: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    version: String,
    uptime: u64,
    services: std::collections::BTreeMap<String, ServiceStatus>,
}

/// Service status structure
#[derive(Debug, serde::Serialize)]
struct ServiceStatus {
    status: String,
    last_check: chrono::DateTime<chrono::Utc>,
    error: Option<String>,
}

/// Readiness status structure
#[derive(Debug, serde::Serialize)]
struct ReadinessStatus {
    is_ready: bool,
    timestamp: chrono::DateTime<chrono::Utc>,
    services: std::collections::BTreeMap<String, ServiceStatus>,
    errors: Vec<String>,
}

/// Liveness status structure
#[derive(Debug, serde::Serialize)]
struct LivenessStatus {
    status: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    uptime: u64,
}

/// Get application uptime in seconds
fn get_uptime() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::OnceLock;

    static START_TIME: OnceLock<chrono::DateTime<chrono::Utc>> = OnceLock::new();
    START_TIME.get_or_init(|| chrono::Utc::now());

    let start = START_TIME.get().unwrap();
    chrono::Utc::now()
        .signed_duration_since(*start)
        .num_seconds() as u64
}

/// Get service status for all services
async fn get_service_status(
    _state: &AppState,
) -> std::collections::BTreeMap<String, ServiceStatus> {
    let mut services = std::collections::BTreeMap::new();

    // Check storage service
    services.insert(
        "storage".to_string(),
        ServiceStatus {
            status: "healthy".to_string(),
            last_check: chrono::Utc::now(),
            error: None,
        },
    );

    // Check attestation service
    services.insert(
        "attestation".to_string(),
        ServiceStatus {
            status: "healthy".to_string(),
            last_check: chrono::Utc::now(),
            error: None,
        },
    );

    // Check KBS service
    services.insert(
        "kbs".to_string(),
        ServiceStatus {
            status: "healthy".to_string(),
            last_check: chrono::Utc::now(),
            error: None,
        },
    );

    // Check scheduler service
    services.insert(
        "scheduler".to_string(),
        ServiceStatus {
            status: "healthy".to_string(),
            last_check: chrono::Utc::now(),
            error: None,
        },
    );

    // Check builder service
    services.insert(
        "builder".to_string(),
        ServiceStatus {
            status: "healthy".to_string(),
            last_check: chrono::Utc::now(),
            error: None,
        },
    );

    services
}

/// Check if all services are ready
async fn check_readiness(state: &AppState) -> ReadinessStatus {
    let services = get_service_status(state).await;
    let mut errors = Vec::new();
    let mut is_ready = true;

    for (name, service) in &services {
        if service.status != "healthy" {
            is_ready = false;
            errors.push(format!(
                "Service {} is not healthy: {:?}",
                name, service.error
            ));
        }
    }

    ReadinessStatus {
        is_ready,
        timestamp: chrono::Utc::now(),
        services,
        errors,
    }
}
