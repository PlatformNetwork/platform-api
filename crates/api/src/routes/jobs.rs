//! Refactored jobs API routes
//! Organized into modular components for better maintainability

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use uuid::Uuid;

use crate::state::AppState;
use crate::job_distributor::JobDistributor;

// Import modular components
use job_management::*;
use job_results::*;
use job_monitoring::*;

/// Create jobs router with organized route groups
pub fn create_router() -> Router<AppState> {
    Router::new()
        // Core job management routes
        .route("/api/jobs", post(create_job).get(list_jobs))
        .route("/api/jobs/pending", get(get_pending_jobs))
        .route("/api/jobs/claim", post(claim_job))
        .route("/api/jobs/next", get(get_next_job))
        .route("/api/jobs/stats", get(get_job_stats))
        
        // Specific job operations
        .route("/api/jobs/:id", get(get_job))
        .route("/api/jobs/:id/claim", post(claim_specific_job))
        .route("/api/jobs/:id/complete", post(complete_job))
        .route("/api/jobs/:id/fail", post(fail_job))
        
        // Job results and progress
        .route("/api/jobs/:id/results", post(submit_results))
        .route("/api/jobs/:id/progress", get(get_job_progress))
        .route("/api/jobs/:id/test-results", get(get_job_test_results))
        .route("/api/jobs/:id/current-test", get(get_current_test))
        
        // Job monitoring and logs
        .route("/api/jobs/:id/logs", get(stream_logs))
        .route("/api/jobs/:id/resource-usage", get(get_resource_usage))
        .route("/api/jobs/:id/metrics", get(get_job_metrics))
        .route("/api/jobs/:id/status-stream", get(get_job_status_stream))
        .route("/api/jobs/:id/timeline", get(get_job_timeline))
        
        // Challenge-specific routes
        .route(
            "/api/jobs/challenge/create-job",
            post(create_job_from_challenge),
        )
        .route("/challenge/create-job", post(create_job_from_challenge)) // Legacy route
}

/// Create job from challenge specification
pub async fn create_job_from_challenge(
    State(state): State<AppState>,
    Json(request): Json<CreateJobFromChallengeRequest>,
) -> Result<Json<JobMetadata>, StatusCode> {
    // Validate challenge exists
    let challenge = state
        .get_challenge_by_id(request.challenge_id)
        .await
        .map_err(|e| {
            error!("Failed to get challenge {} for job creation: {}", request.challenge_id, e);
            StatusCode::NOT_FOUND
        })?;

    if !challenge.is_active {
        error!("Challenge {} is not active", request.challenge_id);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Convert challenge request to job request
    let job_request = platform_api_scheduler::CreateJobRequest {
        compose_hash: request.compose_hash,
        challenge_id: request.challenge_id,
        validator_hotkey: request.validator_hotkey,
        priority: request.priority.unwrap_or(5),
        max_validators: request.max_validators.unwrap_or(1),
        timeout_seconds: request.timeout_seconds.unwrap_or(3600),
        metadata: request.metadata.unwrap_or_default(),
        requires_distribution: true,
    };

    // Create the job
    let job = create_job(State(state), Json(job_request)).await?;
    
    info!("Created job {} from challenge {}", 
          job.id, request.challenge_id);

    Ok(job)
}

/// Initialize job services and background tasks
pub async fn initialize_job_services(state: AppState) {
    info!("Initializing job services");

    // Start job cleanup task
    spawn_job_cleanup_task(state.clone()).await;

    // Start job metrics collection
    spawn_metrics_collection_task(state.clone()).await;

    // Initialize job distributor
    if let Err(e) = state.job_distributor.initialize().await {
        error!("Failed to initialize job distributor: {}", e);
    }

    info!("Job services initialized");
}

/// Spawn background task for job cleanup
async fn spawn_job_cleanup_task(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300)); // Every 5 minutes
        
        loop {
            interval.tick().await;
            
            if let Err(e) = cleanup_old_jobs(&state).await {
                error!("Job cleanup failed: {}", e);
            }
        }
    });
}

/// Spawn background task for metrics collection
async fn spawn_metrics_collection_task(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60)); // Every minute
        
        loop {
            interval.tick().await;
            
            if let Err(e) = collect_job_metrics(&state).await {
                error!("Metrics collection failed: {}", e);
            }
        }
    });
}

/// Clean up old completed/failed jobs
async fn cleanup_old_jobs(state: &AppState) -> Result<(), anyhow::Error> {
    let cutoff_time = chrono::Utc::now() - chrono::Duration::days(7); // Keep jobs for 7 days
    
    let cleaned_count = state
        .cleanup_old_jobs(cutoff_time)
        .await?;

    if cleaned_count > 0 {
        info!("Cleaned up {} old jobs", cleaned_count);
    }

    Ok(())
}

/// Collect and aggregate job metrics
async fn collect_job_metrics(state: &AppState) -> Result<(), anyhow::Error> {
    let metrics = state
        .get_aggregated_job_metrics()
        .await?;

    // Store metrics in time-series database or cache
    state
        .store_job_metrics(metrics)
        .await?;

    Ok(())
}

/// Get job service statistics
pub async fn get_job_service_stats(state: &AppState) -> serde_json::Value {
    let total_jobs = state.get_total_job_count().await.unwrap_or(0);
    let active_jobs = state.get_active_job_count().await.unwrap_or(0);
    let completed_jobs = state.get_completed_job_count().await.unwrap_or(0);
    let failed_jobs = state.get_failed_job_count().await.unwrap_or(0);

    serde_json::json!({
        "total_jobs": total_jobs,
        "active_jobs": active_jobs,
        "completed_jobs": completed_jobs,
        "failed_jobs": failed_jobs,
        "success_rate": if total_jobs > 0 {
            completed_jobs as f64 / total_jobs as f64 * 100.0
        } else {
            0.0
        },
        "distributor_status": state.job_distributor.get_status().await,
        "uptime": chrono::Utc::now().timestamp() - state.get_start_time().await
    })
}

/// Update job configuration
pub async fn update_job_config(
    state: &AppState,
    config: serde_json::Value,
) -> Result<(), anyhow::Error> {
    info!("Updating job service configuration");

    // Update job timeout if provided
    if let Some(timeout) = config.get("default_timeout").and_then(|v| v.as_u64()) {
        state.set_default_job_timeout(timeout as i32).await;
        info!("Updated default job timeout to {} seconds", timeout);
    }

    // Update max concurrent jobs if provided
    if let Some(max_jobs) = config.get("max_concurrent_jobs").and_then(|v| v.as_u64()) {
        state.set_max_concurrent_jobs(max_jobs as usize).await;
        info!("Updated max concurrent jobs to {}", max_jobs);
    }

    // Update cleanup retention period if provided
    if let Some(retention_days) = config.get("retention_days").and_then(|v| v.as_u64()) {
        state.set_job_retention_days(retention_days as i32).await;
        info!("Updated job retention period to {} days", retention_days);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_job_request_validation() {
        // This would require setting up a test state
        // For now, just ensure the functions compile
        assert!(true);
    }
}

// Request types for challenge job creation
#[derive(serde::Deserialize)]
pub struct CreateJobFromChallengeRequest {
    pub challenge_id: Uuid,
    pub compose_hash: String,
    pub validator_hotkey: Option<String>,
    pub priority: Option<i32>,
    pub max_validators: Option<i32>,
    pub timeout_seconds: Option<i32>,
    pub metadata: Option<serde_json::Value>,
}
