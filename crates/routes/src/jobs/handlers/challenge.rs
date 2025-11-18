use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use platform_api::job_distributor::{DistributeJobRequest, JobDistributor};
use platform_api::state::AppState;
use platform_api_models::JobMetadata;
use platform_api_scheduler::CreateJobRequest;

use crate::jobs::types::ChallengeCreateJobRequest;

/// Create a job from challenge SDK
pub async fn create_job_from_challenge(
    State(state): State<AppState>,
    Json(request): Json<ChallengeCreateJobRequest>,
) -> Result<Json<JobMetadata>, StatusCode> {
    // Parse priority
    let priority = request.priority.as_ref().map(|p| match p.as_str() {
        "low" => platform_api_models::JobPriority::Low,
        "high" => platform_api_models::JobPriority::High,
        "critical" => platform_api_models::JobPriority::Critical,
        _ => platform_api_models::JobPriority::Normal,
    });

    // Resolve challenge_id: try UUID first, then lookup by name
    let challenge_uuid = if let Ok(uuid) = Uuid::parse_str(&request.challenge_id) {
        uuid
    } else {
        // Try to find challenge by name in database
        if let Some(pool) = &state.database_pool {
            let result: Option<Uuid> =
                sqlx::query_scalar("SELECT id FROM challenges WHERE name = $1 LIMIT 1")
                    .bind(&request.challenge_id)
                    .fetch_optional(pool.as_ref())
                    .await
                    .ok()
                    .flatten();

            match result {
                Some(uuid) => uuid,
                None => {
                    tracing::error!(
                        "Challenge not found by name or UUID: {}",
                        request.challenge_id
                    );
                    return Err(StatusCode::BAD_REQUEST);
                }
            }
        } else {
            tracing::error!(
                "Database pool not available and challenge_id is not a UUID: {}",
                request.challenge_id
            );
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    // Create scheduler request
    let create_request = CreateJobRequest {
        challenge_id: platform_api_models::Id::from(challenge_uuid),
        payload: request.payload.clone(),
        priority,
        runtime: platform_api_models::RuntimeType::Docker,
        timeout: request.timeout,
        max_retries: request.max_retries,
    };

    // Create the job in the scheduler
    let job = state
        .scheduler
        .create_job(create_request)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create job from challenge: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Try to get challenge info and distribute job
    let challenge_id = request.challenge_id.clone();

    // Try to get compose_hash from challenge registry (by UUID or by name)
    let compose_hash = {
        let registry = state.challenge_registry.read().await;
        registry
            .values()
            .find(|spec| spec.id == challenge_uuid || spec.name == challenge_id)
            .map(|spec| spec.compose_hash.clone())
            .unwrap_or_else(|| "unknown".to_string())
    };

    if compose_hash != "unknown" {
        // Create job distributor
        let distributor = JobDistributor::new(state.clone());

        // Prepare distribution request
        let distribute_request = DistributeJobRequest {
            job_id: job.id.to_string(),
            job_name: request.job_name.clone(),
            payload: request.payload.clone(),
            compose_hash,
            challenge_id: challenge_id.clone(),
            challenge_cvm_ws_url: None,
        };

        // Distribute job to validators
        match distributor
            .distribute_job_to_validators(distribute_request)
            .await
        {
            Ok(response) => {
                tracing::info!(
                    "Challenge {} created and distributed job {} to {} validators",
                    challenge_id,
                    job.id,
                    response.assigned_validators.len()
                );
            }
            Err(e) => {
                tracing::error!(
                    "Failed to distribute job {} from challenge {}: {}",
                    job.id,
                    challenge_id,
                    e
                );
            }
        }
    } else {
        tracing::warn!(
            "Could not find challenge {} (UUID: {}) to distribute job {}",
            challenge_id,
            challenge_uuid,
            job.id
        );
    }

    Ok(Json(job))
}

