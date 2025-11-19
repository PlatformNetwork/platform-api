//! Core job management operations (create, list, get, claim, complete, fail)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

use crate::job_distributor::{DistributeJobRequest, JobDistributor};
use crate::state::AppState;
use platform_api_models::{
    ClaimJobRequest, ClaimJobResponse, JobListResponse, JobMetadata, JobStats,
};
use platform_api_scheduler::CreateJobRequest;

/// Create a new job
pub async fn create_job(
    State(state): State<AppState>,
    Json(request): Json<CreateJobRequest>,
) -> Result<Json<JobMetadata>, StatusCode> {
    // Clone the request data we need before moving it
    let compose_hash = request.compose_hash.clone();
    let challenge_id = request.challenge_id.clone();
    
    // Validate request
    if let Err(e) = validate_create_job_request(&request).await {
        error!("Invalid job creation request: {}", e);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Create job through scheduler
    let job = state
        .scheduler
        .create_job(request)
        .await
        .map_err(|e| {
            error!("Failed to create job: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Distribute job to validators if needed
    if job.requires_distribution {
        let distribute_request = DistributeJobRequest {
            job_id: job.id,
            compose_hash: compose_hash.clone(),
            challenge_id: challenge_id.clone(),
            priority: job.priority,
            max_validators: job.max_validators,
        };

        if let Err(e) = state.job_distributor.distribute_job(distribute_request).await {
            error!("Failed to distribute job {}: {}", job.id, e);
            // Don't fail the request, just log the error
        }
    }

    info!("Created job {} for challenge {} with compose hash {}", 
          job.id, challenge_id, compose_hash);

    Ok(Json(job))
}

/// List jobs with optional filtering
pub async fn list_jobs(
    State(state): State<AppState>,
    Query(params): Query<ListJobsQuery>,
) -> Result<Json<JobListResponse>, StatusCode> {
    let limit = params.limit.unwrap_or(50).min(100); // Max 100 jobs
    let offset = params.offset.unwrap_or(0);

    let jobs = state
        .scheduler
        .list_jobs(limit, offset, params.status, params.challenge_id)
        .await
        .map_err(|e| {
            error!("Failed to list jobs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let total = state
        .scheduler
        .count_jobs(params.status, params.challenge_id)
        .await
        .map_err(|e| {
            error!("Failed to count jobs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(JobListResponse {
        jobs,
        total,
        limit,
        offset,
    }))
}

/// Get specific job by ID
pub async fn get_job(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<JobMetadata>, StatusCode> {
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {}: {}", job_id, e);
            StatusCode::NOT_FOUND
        })?;

    Ok(Json(job))
}

/// Claim a job (for validators)
pub async fn claim_job(
    State(state): State<AppState>,
    Json(request): Json<ClaimJobRequest>,
) -> Result<Json<ClaimJobResponse>, StatusCode> {
    // Validate validator
    if let Err(e) = validate_validator_for_claim(&request.validator_hotkey, &state).await {
        error!("Validator validation failed for job claim: {}", e);
        return Err(StatusCode::FORBIDDEN);
    }

    let claim_response = state
        .scheduler
        .claim_job(request)
        .await
        .map_err(|e| {
            error!("Failed to claim job: {}", e);
            match e.downcast_ref::<platform_api_scheduler::SchedulerError>() {
                Some(platform_api_scheduler::SchedulerError::JobNotFound) => StatusCode::NOT_FOUND,
                Some(platform_api_scheduler::SchedulerError::JobAlreadyClaimed) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?;

    info!("Job {} claimed by validator {}", 
          claim_response.job_id, claim_response.validator_hotkey);

    Ok(Json(claim_response))
}

/// Claim a specific job by ID
pub async fn claim_specific_job(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(request): Json<ClaimJobRequest>,
) -> Result<Json<ClaimJobResponse>, StatusCode> {
    // Validate validator
    if let Err(e) = validate_validator_for_claim(&request.validator_hotkey, &state).await {
        error!("Validator validation failed for specific job claim: {}", e);
        return Err(StatusCode::FORBIDDEN);
    }

    let mut request = request;
    request.job_id = Some(job_id);

    let claim_response = state
        .scheduler
        .claim_job(request)
        .await
        .map_err(|e| {
            error!("Failed to claim specific job {}: {}", job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    info!("Specific job {} claimed by validator {}", 
          job_id, claim_response.validator_hotkey);

    Ok(Json(claim_response))
}

/// Complete a job successfully
pub async fn complete_job(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(request): Json<CompleteJobRequest>,
) -> Result<(), StatusCode> {
    // Validate job completion request
    if let Err(e) = validate_complete_job_request(&request).await {
        error!("Invalid job completion request: {}", e);
        return Err(StatusCode::BAD_REQUEST);
    }

    state
        .scheduler
        .complete_job(job_id, request.validator_hotkey, request.results)
        .await
        .map_err(|e| {
            error!("Failed to complete job {}: {}", job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    info!("Job {} completed successfully", job_id);
    Ok(())
}

/// Mark a job as failed
pub async fn fail_job(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(request): Json<FailJobRequest>,
) -> Result<(), StatusCode> {
    // Validate job failure request
    if let Err(e) = validate_fail_job_request(&request).await {
        error!("Invalid job failure request: {}", e);
        return Err(StatusCode::BAD_REQUEST);
    }

    state
        .scheduler
        .fail_job(job_id, request.validator_hotkey, request.error_message)
        .await
        .map_err(|e| {
            error!("Failed to mark job {} as failed: {}", job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    warn!("Job {} failed: {}", job_id, request.error_message);
    Ok(())
}

/// Get next available job for a validator
pub async fn get_next_job(
    State(state): State<AppState>,
    Query(params): Query<GetNextJobQuery>,
) -> Result<Json<Option<JobMetadata>>, StatusCode> {
    // Validate validator
    if let Err(e) = validate_validator_for_claim(&params.validator_hotkey, &state).await {
        error!("Validator validation failed for next job request: {}", e);
        return Err(StatusCode::FORBIDDEN);
    }

    let job = state
        .scheduler
        .get_next_job(params.validator_hotkey, params.challenge_id)
        .await
        .map_err(|e| {
            error!("Failed to get next job: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(job))
}

/// Get job statistics
pub async fn get_job_stats(
    State(state): State<AppState>,
    Query(params): Query<JobStatsQuery>,
) -> Result<Json<JobStats>, StatusCode> {
    let stats = state
        .scheduler
        .get_job_stats(params.challenge_id, params.time_range)
        .await
        .map_err(|e| {
            error!("Failed to get job stats: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(stats))
}

/// Get pending jobs
pub async fn get_pending_jobs(
    State(state): State<AppState>,
    Query(params): Query<PendingJobsQuery>,
) -> Result<Json<Vec<JobMetadata>>, StatusCode> {
    let limit = params.limit.unwrap_or(20).min(50); // Max 50 pending jobs

    let jobs = state
        .scheduler
        .get_pending_jobs(limit, params.challenge_id)
        .await
        .map_err(|e| {
            error!("Failed to get pending jobs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(jobs))
}

// Request/Response types
#[derive(Deserialize)]
pub struct ListJobsQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub status: Option<String>,
    pub challenge_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct GetNextJobQuery {
    pub validator_hotkey: String,
    pub challenge_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct JobStatsQuery {
    pub challenge_id: Option<Uuid>,
    pub time_range: Option<String>, // e.g., "1h", "24h", "7d"
}

#[derive(Deserialize)]
pub struct PendingJobsQuery {
    pub limit: Option<u32>,
    pub challenge_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct CompleteJobRequest {
    pub validator_hotkey: String,
    pub results: serde_json::Value,
}

#[derive(Deserialize)]
pub struct FailJobRequest {
    pub validator_hotkey: String,
    pub error_message: String,
}

// Validation functions
async fn validate_create_job_request(request: &CreateJobRequest) -> Result<(), anyhow::Error> {
    if request.compose_hash.is_empty() {
        return Err(anyhow::anyhow!("Compose hash cannot be empty"));
    }

    if request.challenge_id.is_nil() {
        return Err(anyhow::anyhow!("Challenge ID is required"));
    }

    if request.priority > 10 {
        return Err(anyhow::anyhow!("Priority cannot exceed 10"));
    }

    Ok(())
}

async fn validate_validator_for_claim(validator_hotkey: &str, state: &AppState) -> Result<(), anyhow::Error> {
    if validator_hotkey.is_empty() {
        return Err(anyhow::anyhow!("Validator hotkey cannot be empty"));
    }

    // Check if validator exists and is active
    let validator = state
        .get_validator_by_hotkey(validator_hotkey)
        .await
        .map_err(|e| anyhow::anyhow!("Validator lookup failed: {}", e))?;

    if !validator.is_active {
        return Err(anyhow::anyhow!("Validator is not active"));
    }

    Ok(())
}

async fn validate_complete_job_request(request: &CompleteJobRequest) -> Result<(), anyhow::Error> {
    if request.validator_hotkey.is_empty() {
        return Err(anyhow::anyhow!("Validator hotkey cannot be empty"));
    }

    // Validate results format if needed
    if request.results.is_null() {
        return Err(anyhow::anyhow!("Job results cannot be null"));
    }

    Ok(())
}

async fn validate_fail_job_request(request: &FailJobRequest) -> Result<(), anyhow::Error> {
    if request.validator_hotkey.is_empty() {
        return Err(anyhow::anyhow!("Validator hotkey cannot be empty"));
    }

    if request.error_message.is_empty() {
        return Err(anyhow::anyhow!("Error message cannot be empty"));
    }

    if request.error_message.len() > 1000 {
        return Err(anyhow::anyhow!("Error message too long (max 1000 characters)"));
    }

    Ok(())
}
