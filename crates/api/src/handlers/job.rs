use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use crate::routes::jobs::{FailJobRequest, GetNextJobParams, ListJobsParams};
use crate::state::AppState;
use platform_api_models::{
    ClaimJobRequest, ClaimJobResponse, JobListResponse, JobMetadata, JobStats, PlatformResult,
    SubmitResultRequest,
};

/// List jobs handler
pub async fn list_jobs_handler(
    state: State<AppState>,
    params: Query<ListJobsParams>,
) -> PlatformResult<Json<JobListResponse>> {
    let jobs = state
        .scheduler
        .list_jobs(
            params.page.unwrap_or(1),
            params.per_page.unwrap_or(20),
            params.status.clone(),
            params.challenge_id,
        )
        .await?;

    Ok(Json(jobs))
}

/// Get job handler
pub async fn get_job_handler(
    state: State<AppState>,
    id: Path<Uuid>,
) -> PlatformResult<Json<JobMetadata>> {
    let job = state.scheduler.get_job(*id).await?;
    Ok(Json(job))
}

/// Claim job handler
pub async fn claim_job_handler(
    state: State<AppState>,
    request: Json<ClaimJobRequest>,
) -> PlatformResult<Json<ClaimJobResponse>> {
    let response = state.scheduler.claim_job(request.0).await?;
    Ok(Json(response))
}

/// Claim specific job handler
pub async fn claim_specific_job_handler(
    state: State<AppState>,
    id: Path<Uuid>,
    request: Json<ClaimJobRequest>,
) -> PlatformResult<Json<ClaimJobResponse>> {
    let response = state.scheduler.claim_specific_job(*id, request.0).await?;
    Ok(Json(response))
}

/// Complete job handler
pub async fn complete_job_handler(
    state: State<AppState>,
    id: Path<Uuid>,
    request: Json<SubmitResultRequest>,
) -> PlatformResult<StatusCode> {
    state.scheduler.complete_job(*id, request.0).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Fail job handler
pub async fn fail_job_handler(
    state: State<AppState>,
    id: Path<Uuid>,
    request: Json<FailJobRequest>,
) -> PlatformResult<StatusCode> {
    let fail_request = platform_api_models::FailJobRequest {
        reason: request.reason.clone(),
        error_details: request.error_details.clone(),
    };
    state.scheduler.fail_job(*id, fail_request).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get next job handler
pub async fn get_next_job_handler(
    state: State<AppState>,
    params: Query<GetNextJobParams>,
) -> PlatformResult<Json<Option<ClaimJobResponse>>> {
    let job = state
        .scheduler
        .get_next_job(params.validator_hotkey.clone(), params.runtime.clone())
        .await?;

    Ok(Json(job))
}

/// Get job stats handler
pub async fn get_job_stats_handler(state: State<AppState>) -> PlatformResult<Json<JobStats>> {
    let stats = state.scheduler.get_job_stats().await?;
    Ok(Json(stats))
}
