use axum::{extract::State, http::StatusCode, response::Json, routing::post, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use platform_api::state::AppState;

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
    state: State<AppState>,
    Json(req): Json<ResultSubmitRequest>,
) -> Result<Json<ResultSubmitResponse>, StatusCode> {
    // Try to extract job_id from session_token (format: "job_id:submission_id" or just job_id)
    let job_id_str = req.session_token.split(':').next().unwrap_or(&req.session_token);
    
    let job_id = match uuid::Uuid::parse_str(job_id_str) {
        Ok(id) => id,
        Err(_) => {
            tracing::warn!("Invalid job_id in session_token: {}", job_id_str);
            // Try to find job by session_token in payload
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    // Convert ResultSubmitRequest to EvalResult format expected by scheduler
    use std::collections::BTreeMap;
    let metrics_clone = req.metrics.clone();
    let eval_result = platform_api_models::EvalResult {
        job_id,
        submission_id: uuid::Uuid::parse_str(&req.session_token)
            .unwrap_or_else(|_| uuid::Uuid::new_v4()),
        scores: {
            let mut scores = BTreeMap::new();
            scores.insert("score".to_string(), req.score);
            // Add other metrics as scores
            for (key, value) in metrics_clone.clone() {
                scores.insert(key, value);
            }
            scores
        },
        metrics: metrics_clone.clone(),
        logs: req.logs.clone(),
        error: req.error.clone(),
        execution_time: metrics_clone
            .get("execution_time_ms")
            .or_else(|| metrics_clone.get("execution_time"))
            .map(|v| *v as u64)
            .unwrap_or(0),
        resource_usage: platform_api_models::ResourceUsage {
            cpu_time: metrics_clone.get("cpu_time").map(|v| *v as u64).unwrap_or(0),
            memory_peak: metrics_clone.get("memory_peak").map(|v| *v as u64).unwrap_or(0),
            disk_usage: metrics_clone.get("disk_usage").map(|v| *v as u64).unwrap_or(0),
            network_bytes: metrics_clone.get("network_bytes").map(|v| *v as u64).unwrap_or(0),
        },
        attestation_receipt: None,
    };

    let submit_request = platform_api_models::SubmitResultRequest {
        job_id,
        result: eval_result,
        receipts: vec![format!("result:{}:{}", req.session_token, Utc::now())],
    };

    // Complete the job via scheduler
    match state.scheduler.complete_job(job_id, submit_request).await {
        Ok(_) => {
            tracing::info!("Job {} completed successfully", job_id);
            let receipt = format!("result:{}:{}", req.session_token, Utc::now());
            Ok(Json(ResultSubmitResponse {
                stored: true,
                receipt,
            }))
        }
        Err(e) => {
            tracing::error!("Failed to complete job {}: {}", job_id, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
