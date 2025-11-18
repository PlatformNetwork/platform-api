use axum::{extract::State, http::StatusCode, response::Json, routing::post, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub timestamp: i64,
    pub status: String,
    pub metrics: ExecutionMetrics,
}

#[derive(Debug, Deserialize)]
pub struct ExecutionMetrics {
    pub cpu_usage_percent: f64,
    pub memory_usage_mb: u64,
    pub execution_time_sec: u64,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatResponse {
    pub accepted: bool,
}

#[derive(Debug, Deserialize)]
pub struct LogEntry {
    pub timestamp: i64,
    pub level: String,
    pub message: String,
    pub component: String,
}

#[derive(Debug, Serialize)]
pub struct LogResponse {
    pub accepted: bool,
}

#[derive(Debug, Deserialize)]
pub struct ResultSubmitRequest {
    pub session_token: String,
    pub score: f64,
    pub metrics: std::collections::BTreeMap<String, f64>,
    pub logs: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResultSubmitResponse {
    pub stored: bool,
    pub receipt: String,
}

pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/results/heartbeat", post(heartbeat))
        .route("/results/logs", post(log_entry))
        .route("/results/submit", post(submit_result))
}

pub async fn heartbeat(
    _state: State<AppState>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, StatusCode> {
    tracing::info!(
        "Heartbeat received: status={}, cpu={}%, memory={}MB",
        req.status,
        req.metrics.cpu_usage_percent,
        req.metrics.memory_usage_mb
    );
    Ok(Json(HeartbeatResponse { accepted: true }))
}

pub async fn log_entry(
    _state: State<AppState>,
    Json(entry): Json<LogEntry>,
) -> Result<Json<LogResponse>, StatusCode> {
    match entry.level.as_str() {
        "error" => tracing::error!("[{}] {}", entry.component, entry.message),
        "warn" => tracing::warn!("[{}] {}", entry.component, entry.message),
        "info" => tracing::info!("[{}] {}", entry.component, entry.message),
        _ => tracing::debug!("[{}] {}", entry.component, entry.message),
    }
    Ok(Json(LogResponse { accepted: true }))
}

pub async fn submit_result(
    _state: State<AppState>,
    Json(req): Json<ResultSubmitRequest>,
) -> Result<Json<ResultSubmitResponse>, StatusCode> {
    let receipt = format!("result:{}:{}", req.session_token, Utc::now());
    Ok(Json(ResultSubmitResponse {
        stored: true,
        receipt,
    }))
}
