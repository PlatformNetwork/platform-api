use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use platform_api::state::AppState;
use platform_api_models::SubmitResultRequest;

use crate::jobs::types::FailJobRequest;

/// Complete job with results
pub async fn complete_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<SubmitResultRequest>,
) -> Result<StatusCode, StatusCode> {
    state
        .scheduler
        .complete_job(id, request)
        .await
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
    state
        .scheduler
        .fail_job(id, fail_request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Submit job results (alias for complete_job)
pub async fn submit_results(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<SubmitResultRequest>,
) -> Result<StatusCode, StatusCode> {
    // Complete job in scheduler
    let eval_result = request.result.clone();
    state
        .scheduler
        .complete_job(id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Also forward result to challenge if job was distributed
    let job_id_str = id.to_string();
    let job_cache = {
        let cache = state.job_cache.read().await;
        cache.get(&job_id_str).cloned()
    };

    if let Some(cache) = job_cache {
        // Convert SubmitResultRequest to JobResult format
        let result_value = serde_json::json!({
            "scores": eval_result.scores,
            "metrics": eval_result.metrics,
            "logs": eval_result.logs,
            "execution_time": eval_result.execution_time,
            "job_type": "evaluate_agent",
        });

        let job_result = platform_api::job_distributor::JobResult {
            job_id: job_id_str.clone(),
            result: result_value,
            error: eval_result.error.clone(),
            validator_hotkey: cache.assigned_validators.first().cloned(),
        };

        // Forward to challenge (non-blocking, log errors but don't fail the request)
        let distributor = platform_api::job_distributor::JobDistributor::new(state.clone());
        if let Err(e) = distributor.forward_job_result(job_result).await {
            tracing::warn!(
                job_id = &job_id_str,
                error = %e,
                "Failed to forward job result to challenge (job still marked as completed)"
            );
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

