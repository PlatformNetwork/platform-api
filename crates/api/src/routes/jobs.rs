use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post, put},
    Router,
};
use serde::Deserialize;
use uuid::Uuid;

use platform_api_models::{
    ClaimJobRequest, ClaimJobResponse, SubmitResultRequest, 
    JobListResponse, JobStats, JobMetadata
};
use platform_api_scheduler::CreateJobRequest;
use crate::state::AppState;

/// Create jobs router
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/jobs", post(create_job).get(list_jobs))
        .route("/jobs/pending", get(get_pending_jobs))
        .route("/jobs/claim", post(claim_job))
        .route("/jobs/:id", get(get_job))
        .route("/jobs/:id/claim", post(claim_specific_job))
        .route("/jobs/:id/complete", post(complete_job))
        .route("/jobs/:id/results", post(submit_results))
        .route("/jobs/:id/fail", post(fail_job))
        .route("/jobs/next", get(get_next_job))
        .route("/jobs/stats", get(get_job_stats))
}

/// Create a new job
pub async fn create_job(
    State(state): State<AppState>,
    Json(request): Json<CreateJobRequest>,
) -> Result<Json<JobMetadata>, StatusCode> {
    let job = state.scheduler.create_job(request).await
        .map_err(|e| {
            tracing::error!("Failed to create job: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(job))
}

/// List jobs with pagination
pub async fn list_jobs(
    State(state): State<AppState>,
    Query(params): Query<ListJobsParams>,
) -> Result<Json<JobListResponse>, StatusCode> {
    let jobs = state.scheduler.list_jobs(
        params.page.unwrap_or(1),
        params.per_page.unwrap_or(20),
        params.status,
        params.challenge_id,
    ).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(jobs))
}

/// Get job details
pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobMetadata>, StatusCode> {
    let job = state.scheduler.get_job(id).await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(job))
}

/// Claim next available job
pub async fn claim_job(
    State(state): State<AppState>,
    Json(request): Json<ClaimJobRequest>,
) -> Result<Json<ClaimJobResponse>, StatusCode> {
    let response = state.scheduler.claim_job(request).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(response))
}

/// Claim specific job
pub async fn claim_specific_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<ClaimJobRequest>,
) -> Result<Json<ClaimJobResponse>, StatusCode> {
    let response = state.scheduler.claim_specific_job(id, request).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(response))
}

/// Complete job with results
pub async fn complete_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<SubmitResultRequest>,
) -> Result<StatusCode, StatusCode> {
    state.scheduler.complete_job(id, request).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Fail job
pub async fn fail_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<FailJobRequest>,
) -> Result<StatusCode, StatusCode> {
    let fail_request = platform_api_models::FailJobRequest {
        reason: request.reason.clone(),
        error_details: request.error_details.clone(),
    };
    state.scheduler.fail_job(id, fail_request).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Get next available job for validator
pub async fn get_next_job(
    State(state): State<AppState>,
    Query(params): Query<GetNextJobParams>,
) -> Result<Json<Option<ClaimJobResponse>>, StatusCode> {
    let job = state.scheduler.get_next_job(
        params.validator_hotkey,
        params.runtime,
    ).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(job))
}

/// Get job statistics
pub async fn get_job_stats(
    State(state): State<AppState>,
) -> Result<Json<JobStats>, StatusCode> {
    let stats = state.scheduler.get_job_stats().await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(stats))
}

/// Get pending jobs for validator
pub async fn get_pending_jobs(
    State(state): State<AppState>,
    Query(params): Query<PendingJobsParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let jobs = state.scheduler.list_jobs(
        1,
        100,
        Some("pending".to_string()),
        None,
    ).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "jobs": jobs.jobs.iter().map(|j| serde_json::json!({
            "id": j.id,
            "challenge_id": j.challenge_id,
            "status": j.status,
        })).collect::<Vec<_>>()
    })))
}

/// Submit job results (alias for complete_job)
pub async fn submit_results(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<SubmitResultRequest>,
) -> Result<StatusCode, StatusCode> {
    state.scheduler.complete_job(id, request).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Query parameters for listing jobs
#[derive(Debug, Deserialize)]
pub struct ListJobsParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub status: Option<String>,
    pub challenge_id: Option<Uuid>,
}

/// Query parameters for getting next job
#[derive(Debug, Deserialize)]
pub struct GetNextJobParams {
    pub validator_hotkey: String,
    pub runtime: Option<String>,
}

/// Request to fail a job
#[derive(Debug, Clone, serde::Deserialize)]
pub struct FailJobRequest {
    pub reason: String,
    pub error_details: Option<String>,
}

/// Query parameters for pending jobs
#[derive(Debug, Deserialize)]
pub struct PendingJobsParams {
    pub validator_hotkey: Option<String>,
}


