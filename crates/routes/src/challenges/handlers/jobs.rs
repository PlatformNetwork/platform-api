use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use platform_api::state::AppState;

use crate::challenges::types::ChallengeJobsParams;

/// Get all jobs for a challenge with results
pub async fn get_challenge_jobs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<ChallengeJobsParams>,
) -> Result<Json<JsonValue>, StatusCode> {
    // Get jobs for this challenge using scheduler (which uses PostgreSQL)
    let page = params.page.unwrap_or(1);
    let per_page = params.per_page.unwrap_or(20);

    let jobs = state
        .scheduler
        .list_jobs(page, per_page, params.status, Some(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // For each job, get test results count
    let jobs_with_results: Vec<JsonValue> = jobs
        .jobs
        .iter()
        .map(|job| {
            // Count test results for this job (we'll fetch details separately if needed)
            serde_json::json!({
                "job": job,
                "has_test_results": false, // Will be populated if test_results detail is requested
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "challenge_id": id,
        "jobs": jobs_with_results,
        "total": jobs.total,
        "page": jobs.page,
        "per_page": jobs.per_page,
    })))
}

