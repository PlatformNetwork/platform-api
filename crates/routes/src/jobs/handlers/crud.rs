use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use platform_api::job_distributor::{DistributeJobRequest, JobDistributor};
use platform_api::state::AppState;
use platform_api_models::{ClaimJobRequest, ClaimJobResponse, JobListResponse, JobMetadata, JobStats};
use platform_api_scheduler::CreateJobRequest;

use crate::jobs::types::{GetNextJobParams, ListJobsParams, PendingJobsParams};

/// Create a new job
pub async fn create_job(
    State(state): State<AppState>,
    Json(request): Json<CreateJobRequest>,
) -> Result<Json<JobMetadata>, StatusCode> {
    // Clone the request data we need before moving it
    let challenge_id = request.challenge_id;
    let payload = request.payload.clone();

    // Create the job in the scheduler
    let job = state.scheduler.create_job(request).await.map_err(|e| {
        tracing::error!("Failed to create job: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Get challenge info to find compose_hash
    let challenge_uuid = Uuid::parse_str(&challenge_id.to_string()).unwrap_or_else(|_| {
        tracing::warn!(
            "challenge_id '{}' is not a valid UUID",
            challenge_id.to_string()
        );
        Uuid::new_v4()
    });

    // Try to get compose_hash from challenge registry or use placeholder
    let compose_hash = state
        .challenge_registry
        .read()
        .await
        .values()
        .find(|spec| spec.id == challenge_uuid)
        .map(|spec| spec.compose_hash.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // Create job distributor
    let distributor = JobDistributor::new(state.clone());

    // Prepare distribution request
    let distribute_request = DistributeJobRequest {
        job_id: job.id.to_string(),
        job_name: payload
            .get("job_name")
            .and_then(|v| v.as_str())
            .unwrap_or("job")
            .to_string(),
        payload: payload.clone(),
        compose_hash,
        challenge_id: challenge_id.to_string(),
        challenge_cvm_ws_url: None,
    };

    // Distribute job to validators if we found a valid compose_hash
    if distribute_request.compose_hash != "unknown" {
        match distributor
            .distribute_job_to_validators(distribute_request)
            .await
        {
            Ok(response) => {
                tracing::info!(
                    "Distributed job {} to {} validators",
                    job.id,
                    response.assigned_validators.len()
                );
            }
            Err(e) => {
                tracing::error!("Failed to distribute job {}: {}", job.id, e);
            }
        }
    } else {
        tracing::warn!(
            "Could not find challenge {} to distribute job {}",
            challenge_id,
            job.id
        );
    }

    Ok(Json(job))
}

/// List jobs with pagination
pub async fn list_jobs(
    State(state): State<AppState>,
    Query(params): Query<ListJobsParams>,
) -> Result<Json<JobListResponse>, StatusCode> {
    let jobs = state
        .scheduler
        .list_jobs(
            params.page.unwrap_or(1),
            params.per_page.unwrap_or(20),
            params.status,
            params.challenge_id,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(jobs))
}

/// Get job details
pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobMetadata>, StatusCode> {
    let job = state
        .scheduler
        .get_job(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(job))
}

/// Get job statistics
pub async fn get_job_stats(State(state): State<AppState>) -> Result<Json<JobStats>, StatusCode> {
    let stats = state
        .scheduler
        .get_job_stats()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(stats))
}

/// Get pending jobs for validator
pub async fn get_pending_jobs(
    State(state): State<AppState>,
    Query(_params): Query<PendingJobsParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let jobs = state
        .scheduler
        .list_jobs(1, 100, Some("pending".to_string()), None)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list pending jobs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Extract payload information from JobMetadata
    let mut jobs_with_payloads = Vec::new();

    for job in &jobs.jobs {
        let submission_id = job
            .payload
            .as_ref()
            .and_then(|p| p.get("session_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| job.id.to_string());

        let miner_hotkey = job
            .payload
            .as_ref()
            .and_then(|p| p.get("agent_hash"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        jobs_with_payloads.push(serde_json::json!({
            "id": job.id.to_string(),
            "challenge_id": job.challenge_id.to_string(),
            "submission_id": submission_id,
            "miner_hotkey": miner_hotkey,
            "status": format!("{:?}", job.status).to_lowercase(),
            "created_at": job.created_at.to_rfc3339(),
        }));
    }

    Ok(Json(serde_json::json!({
        "jobs": jobs_with_payloads
    })))
}

/// Claim next available job
pub async fn claim_job(
    State(state): State<AppState>,
    Json(request): Json<ClaimJobRequest>,
) -> Result<Json<ClaimJobResponse>, StatusCode> {
    let response = state
        .scheduler
        .claim_job(request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(response))
}

/// Claim specific job
pub async fn claim_specific_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<ClaimJobRequest>,
) -> Result<Json<ClaimJobResponse>, StatusCode> {
    let response = state
        .scheduler
        .claim_specific_job(id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(response))
}

/// Get next available job for validator
pub async fn get_next_job(
    State(state): State<AppState>,
    Query(params): Query<GetNextJobParams>,
) -> Result<Json<Option<ClaimJobResponse>>, StatusCode> {
    let job = state
        .scheduler
        .get_next_job(params.validator_hotkey, params.runtime)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(job))
}

