//! Job monitoring, logging, and resource usage tracking

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Json, Response},
};
use serde::Deserialize;
use uuid::Uuid;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use axum::response::sse::{Event, Sse};

use crate::state::AppState;
use serde_json::Value as JsonValue;

/// Stream job logs in real-time
pub async fn stream_logs(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Query(params): Query<LogStreamQuery>,
) -> Response {
    // Validate job exists
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {} for log streaming: {}", job_id, e);
            StatusCode::NOT_FOUND
        });

    if job.is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Create SSE stream for logs
    let (tx, rx) = mpsc::channel::<Result<Event, anyhow::Error>>(100);

    // Spawn task to stream logs
    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = stream_job_logs_task(job_id, params, tx, state_clone).await {
            error!("Log streaming task failed for job {}: {}", job_id, e);
        }
    });

    // Convert receiver to SSE stream
    let stream = ReceiverStream::new(rx)
        .map(|result| match result {
            Ok(event) => Ok(event),
            Err(e) => Ok(Event::default().data(format!("Error: {}", e))),
        });

    Sse::new(stream).into_response()
}

/// Get job resource usage information
pub async fn get_resource_usage(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Query(params): Query<ResourceUsageQuery>,
) -> Result<Json<ResourceUsageInfo>, StatusCode> {
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {} for resource usage: {}", job_id, e);
            StatusCode::NOT_FOUND
        })?;

    if job.status != "running" && job.status != "completed" {
        error!("Cannot get resource usage for job {} with status: {}", job_id, job.status);
        return Err(StatusCode::BAD_REQUEST);
    }

    let resource_usage = state
        .get_job_resource_usage(job_id, params.time_range)
        .await
        .map_err(|e| {
            error!("Failed to get resource usage for job {}: {}", job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(resource_usage))
}

/// Get job performance metrics
pub async fn get_job_metrics(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<JobMetrics>, StatusCode> {
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {} for metrics: {}", job_id, e);
            StatusCode::NOT_FOUND
        })?;

    let metrics = state
        .get_job_metrics(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job metrics {}: {}", job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(JobMetrics {
        job_id,
        status: job.status,
        created_at: job.created_at,
        started_at: job.started_at,
        completed_at: job.completed_at,
        duration_seconds: metrics.duration_seconds,
        total_tests: metrics.total_tests,
        passed_tests: metrics.passed_tests,
        failed_tests: metrics.failed_tests,
        average_test_duration: metrics.average_test_duration,
        peak_memory_usage: metrics.peak_memory_usage,
        average_cpu_usage: metrics.average_cpu_usage,
        total_network_bytes: metrics.total_network_bytes,
    }))
}

/// Get real-time job status updates
pub async fn get_job_status_stream(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Response {
    // Validate job exists
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {} for status stream: {}", job_id, e);
            StatusCode::NOT_FOUND
        });

    if job.is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Create SSE stream for status updates
    let (tx, rx) = mpsc::channel::<Result<Event, anyhow::Error>>(100);

    // Spawn task to stream status updates
    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = stream_job_status_task(job_id, tx, state_clone).await {
            error!("Status streaming task failed for job {}: {}", job_id, e);
        }
    });

    // Convert receiver to SSE stream
    let stream = ReceiverStream::new(rx)
        .map(|result| match result {
            Ok(event) => Ok(event),
            Err(e) => Ok(Event::default().data(format!("Error: {}", e))),
        });

    Sse::new(stream).into_response()
}

/// Task to stream job logs
async fn stream_job_logs_task(
    job_id: Uuid,
    params: LogStreamQuery,
    tx: mpsc::Sender<Result<Event, anyhow::Error>>,
    state: AppState,
) -> Result<(), anyhow::Error> {
    let mut last_log_id = params.since_id.unwrap_or(0);
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Check for new logs
                let logs = state
                    .get_job_logs_since(job_id, last_log_id, params.limit.unwrap_or(100))
                    .await?;

                for log in logs {
                    let log_event = Event::default()
                        .data(serde_json::to_string(&log)?)
                        .event("log");

                    tx.send(Ok(log_event)).await?;
                    last_log_id = log.id.max(last_log_id);
                }

                // Check if job is completed
                let job = state.scheduler.get_job(job_id).await?;
                if job.status == "completed" || job.status == "failed" {
                    // Send final status and break
                    let final_event = Event::default()
                        .data(format!("Job finished with status: {}", job.status))
                        .event("job_complete");
                    
                    tx.send(Ok(final_event)).await?;
                    break;
                }
            }
            // Handle channel closure
            _ = tx.closed() => {
                info!("Log stream client disconnected for job {}", job_id);
                break;
            }
        }
    }

    Ok(())
}

/// Task to stream job status updates
async fn stream_job_status_task(
    job_id: Uuid,
    tx: mpsc::Sender<Result<Event, anyhow::Error>>,
    state: AppState,
) -> Result<(), anyhow::Error> {
    let mut last_status = String::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Check for status changes
                let job = state.scheduler.get_job(job_id).await?;
                
                if job.status != last_status {
                    let status_event = Event::default()
                        .data(serde_json::json!({
                            "job_id": job_id,
                            "status": job.status,
                            "updated_at": job.updated_at,
                            "progress": job.progress_percentage
                        }).to_string())
                        .event("status_update");

                    tx.send(Ok(status_event)).await?;
                    last_status = job.status.clone();
                }

                // If job is finished, send final update and break
                if job.status == "completed" || job.status == "failed" {
                    let final_event = Event::default()
                        .data(serde_json::json!({
                            "job_id": job_id,
                            "status": job.status,
                            "completed_at": job.completed_at,
                            "final_results": job.results
                        }).to_string())
                        .event("job_complete");
                    
                    tx.send(Ok(final_event)).await?;
                    break;
                }
            }
            // Handle channel closure
            _ = tx.closed() => {
                info!("Status stream client disconnected for job {}", job_id);
                break;
            }
        }
    }

    Ok(())
}

/// Get job execution timeline
pub async fn get_job_timeline(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<Vec<TimelineEvent>>, StatusCode> {
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {} for timeline: {}", job_id, e);
            StatusCode::NOT_FOUND
        })?;

    let timeline = state
        .get_job_timeline(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job timeline {}: {}", job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(timeline))
}

// Request/Response types
#[derive(Deserialize)]
pub struct LogStreamQuery {
    pub since_id: Option<i64>,
    pub limit: Option<i64>,
    pub level: Option<String>, // debug, info, warn, error
}

#[derive(Deserialize)]
pub struct ResourceUsageQuery {
    pub time_range: Option<String>, // e.g., "1h", "24h", "all"
}

#[derive(serde::Serialize)]
pub struct ResourceUsageInfo {
    pub job_id: Uuid,
    pub cpu_usage: Vec<CpuUsagePoint>,
    pub memory_usage: Vec<MemoryUsagePoint>,
    pub disk_io: Vec<DiskIOPoint>,
    pub network_io: Vec<NetworkIOPoint>,
    pub time_range: String,
}

#[derive(serde::Serialize)]
pub struct CpuUsagePoint {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub usage_percentage: f64,
}

#[derive(serde::Serialize)]
pub struct MemoryUsagePoint {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub usage_percentage: f64,
}

#[derive(serde::Serialize)]
pub struct DiskIOPoint {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub read_bytes_per_sec: f64,
    pub write_bytes_per_sec: f64,
}

#[derive(serde::Serialize)]
pub struct NetworkIOPoint {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub bytes_sent_per_sec: f64,
    pub bytes_received_per_sec: f64,
}

#[derive(serde::Serialize)]
pub struct JobMetrics {
    pub job_id: Uuid,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_seconds: Option<i64>,
    pub total_tests: Option<i32>,
    pub passed_tests: Option<i32>,
    pub failed_tests: Option<i32>,
    pub average_test_duration: Option<f64>,
    pub peak_memory_usage: Option<u64>,
    pub average_cpu_usage: Option<f64>,
    pub total_network_bytes: Option<u64>,
}

#[derive(serde::Serialize)]
pub struct TimelineEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub event_type: String,
    pub description: String,
    pub metadata: Option<JsonValue>,
}
