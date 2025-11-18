use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

use crate::job_distributor::{DistributeJobRequest, JobDistributor};
use crate::state::AppState;
use platform_api_models::{
    ClaimJobRequest, ClaimJobResponse, JobListResponse, JobMetadata, JobStats, SubmitResultRequest,
};
use platform_api_scheduler::CreateJobRequest;
use serde_json::Value as JsonValue;

/// Create jobs router
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/api/jobs", post(create_job).get(list_jobs))
        .route("/api/jobs/pending", get(get_pending_jobs))
        .route("/api/jobs/claim", post(claim_job))
        .route("/api/jobs/:id", get(get_job))
        .route("/api/jobs/:id/claim", post(claim_specific_job))
        .route("/api/jobs/:id/complete", post(complete_job))
        .route("/api/jobs/:id/results", post(submit_results))
        .route("/api/jobs/:id/fail", post(fail_job))
        .route("/api/jobs/:id/progress", get(get_job_progress))
        .route("/api/jobs/:id/test-results", get(get_job_test_results))
        .route("/api/jobs/:id/current-test", get(get_current_test))
        .route("/api/jobs/:id/logs", get(stream_logs))
        .route("/api/jobs/:id/resource-usage", get(get_resource_usage))
        .route("/api/jobs/next", get(get_next_job))
        .route("/api/jobs/stats", get(get_job_stats))
        .route(
            "/api/jobs/challenge/create-job",
            post(create_job_from_challenge),
        )
        .route("/challenge/create-job", post(create_job_from_challenge)) // Legacy route for backward compatibility
}

/// Create a new job
pub async fn create_job(
    State(state): State<AppState>,
    Json(request): Json<CreateJobRequest>,
) -> Result<Json<JobMetadata>, StatusCode> {
    // Clone the request data we need before moving it
    let challenge_id = request.challenge_id.clone();
    let payload = request.payload.clone();

    // Create the job in the scheduler
    let job = state.scheduler.create_job(request).await.map_err(|e| {
        tracing::error!("Failed to create job: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Get challenge info to find compose_hash
    let challenge_uuid = Uuid::parse_str(&challenge_id.to_string()).unwrap_or_else(|_| {
        // If challenge_id is not a UUID, try to look it up by name
        // For now, we'll just use a default UUID - this should be improved in production
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
        challenge_cvm_ws_url: None, // This would be set if we had the challenge CVM URL
    };

    // Distribute job to validators if we found a valid compose_hash
    if distribute_request.compose_hash != "unknown" {
        // Distribute job to validators
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
                // Don't fail the request, job is still created
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

    // Extract payload information from JobMetadata (now includes payload field)
    let mut jobs_with_payloads = Vec::new();

    for job in &jobs.jobs {
        // Extract submission_id and miner_hotkey from payload
        let submission_id = job
            .payload
            .as_ref()
            .and_then(|p| p.get("session_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| job.id.to_string()); // Fallback to job id

        let miner_hotkey = job
            .payload
            .as_ref()
            .and_then(|p| p.get("agent_hash"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string()); // Fallback to "unknown"

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

/// Submit job results (alias for complete_job)
pub async fn submit_results(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<SubmitResultRequest>,
) -> Result<StatusCode, StatusCode> {
    // Complete job in scheduler
    let eval_result = request.result.clone(); // Clone EvalResult for forwarding
    state
        .scheduler
        .complete_job(id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Also forward result to challenge if job was distributed
    // Extract validator hotkey from request if available, or use first assigned validator
    let job_id_str = id.to_string();
    let job_cache = {
        let cache = state.job_cache.read().await;
        cache.get(&job_id_str).cloned()
    };

    if let Some(cache) = job_cache {
        // Convert SubmitResultRequest to JobResult format
        // Extract data from EvalResult
        let result_value = serde_json::json!({
            "scores": eval_result.scores,
            "metrics": eval_result.metrics,
            "logs": eval_result.logs,
            "execution_time": eval_result.execution_time,
            "job_type": "evaluate_agent", // Default job type
        });

        let job_result = crate::job_distributor::JobResult {
            job_id: job_id_str.clone(),
            result: result_value,
            error: eval_result.error.clone(),
            validator_hotkey: cache.assigned_validators.first().cloned(),
        };

        // Forward to challenge (non-blocking, log errors but don't fail the request)
        let distributor = crate::job_distributor::JobDistributor::new(state.clone());
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

/// Get real-time job progress from Redis
pub async fn get_job_progress(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JsonValue>, StatusCode> {
    let job_id = id.to_string();

    if let Some(redis) = &state.redis_client {
        // Get raw JSON from Redis for enhanced progress data
        let mut conn = redis
            .client
            .get_tokio_connection_manager()
            .await
            .map_err(|e| {
                tracing::error!("Failed to connect to Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        let key = format!("job:{}:progress", job_id);
        let json_str: Option<String> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get job progress from Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        if let Some(json) = json_str {
            // Parse as generic JSON to preserve all fields
            let progress: JsonValue =
                serde_json::from_str(&json).unwrap_or_else(|_| JsonValue::Null);
            Ok(Json(progress))
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Get detailed test results from PostgreSQL
pub async fn get_job_test_results(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<TestResultsParams>,
) -> Result<Json<JsonValue>, StatusCode> {
    if let Some(pool) = &state.database_pool {
        let limit = params.limit.unwrap_or(1000);

        let rows = sqlx::query(
            r#"
            SELECT id, job_id, challenge_id, task_id, test_name, status,
                   is_resolved, error_message, execution_time_ms,
                   output_text, logs, metrics, created_at
            FROM job_test_results
            WHERE job_id = $1
            ORDER BY created_at ASC
            LIMIT $2
            "#,
        )
        .bind(id)
        .bind(limit as i64)
        .fetch_all(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to query test results: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        let results: Vec<JsonValue> = rows
            .into_iter()
            .map(|row| {
                serde_json::json!({
                    "id": row.get::<Uuid, _>("id"),
                    "job_id": row.get::<Uuid, _>("job_id"),
                    "challenge_id": row.get::<Uuid, _>("challenge_id"),
                    "task_id": row.get::<String, _>("task_id"),
                    "test_name": row.get::<Option<String>, _>("test_name"),
                    "status": row.get::<String, _>("status"),
                    "is_resolved": row.get::<bool, _>("is_resolved"),
                    "error_message": row.get::<Option<String>, _>("error_message"),
                    "execution_time_ms": row.get::<Option<i64>, _>("execution_time_ms"),
                    "output_text": row.get::<Option<String>, _>("output_text"),
                    "logs": row.get::<JsonValue, _>("logs"),
                    "metrics": row.get::<JsonValue, _>("metrics"),
                    "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                })
            })
            .collect();

        Ok(Json(serde_json::json!({
            "job_id": id,
            "test_results": results,
            "total": results.len(),
        })))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Query parameters for test results
#[derive(Debug, Deserialize)]
pub struct TestResultsParams {
    pub limit: Option<u32>,
}

/// Request from challenge to create a job
#[derive(Debug, Deserialize)]
pub struct ChallengeCreateJobRequest {
    pub job_name: String,
    pub payload: JsonValue,
    pub challenge_id: String,
    pub priority: Option<String>,
    pub timeout: Option<u64>,
    pub max_retries: Option<u32>,
}

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
            challenge_cvm_ws_url: None, // Could be enhanced to include the challenge's WebSocket URL
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
                // Don't fail the request, job is still created
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

/// Get currently executing test details
pub async fn get_current_test(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JsonValue>, StatusCode> {
    let job_id = id.to_string();

    if let Some(redis) = &state.redis_client {
        // Get raw JSON from Redis
        let mut conn = redis
            .client
            .get_tokio_connection_manager()
            .await
            .map_err(|e| {
                tracing::error!("Failed to connect to Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        let key = format!("job:{}:progress", job_id);
        let json_str: Option<String> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get job progress from Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        if let Some(json) = json_str {
            // Parse as generic JSON
            let progress: JsonValue =
                serde_json::from_str(&json).unwrap_or_else(|_| JsonValue::Null);

            // Extract current_test from the progress data
            if let Some(current_test) = progress.get("current_test") {
                Ok(Json(current_test.clone()))
            } else {
                // No current test running
                Ok(Json(serde_json::json!({
                    "job_id": job_id,
                    "status": "no_active_test",
                    "message": "No test is currently running"
                })))
            }
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Stream test logs in real-time
pub async fn stream_logs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<LogStreamParams>,
) -> Result<Json<JsonValue>, StatusCode> {
    let job_id = id.to_string();

    if let Some(redis) = &state.redis_client {
        let mut logs = Vec::new();

        // Get connection
        let mut conn = redis
            .client
            .get_tokio_connection_manager()
            .await
            .map_err(|e| {
                tracing::error!("Failed to connect to Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        // Try to read from Redis Streams first
        let stream_key = format!("job:logs:{}", job_id);
        let stream_result: Result<
            Vec<(String, std::collections::HashMap<String, String>)>,
            redis::RedisError,
        > = redis::cmd("XRANGE")
            .arg(&stream_key)
            .arg("-")
            .arg("+")
            .arg("COUNT")
            .arg(params.limit.unwrap_or(1000))
            .query_async(&mut conn)
            .await;

        match stream_result {
            Ok(stream_entries) => {
                // Convert stream entries to log format
                for (_id, fields) in stream_entries {
                    let log_entry = serde_json::json!({
                        "job_id": fields.get("job_id").cloned(),
                        "file": fields.get("file").cloned(),
                        "line": fields.get("line").cloned(),
                        "line_number": fields.get("line_number").and_then(|v| v.parse::<i32>().ok()),
                        "timestamp": fields.get("timestamp").and_then(|v| v.parse::<f64>().ok()),
                    });
                    logs.push(log_entry);
                }
            }
            Err(e) => {
                tracing::debug!(
                    "Failed to read from Redis Streams, falling back to progress data: {}",
                    e
                );

                // Fallback to reading from progress data
                let key = format!("job:{}:progress", job_id);
                let json_str: Option<String> = redis::cmd("GET")
                    .arg(&key)
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to get job progress from Redis: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?;

                if let Some(json) = json_str {
                    let progress: JsonValue =
                        serde_json::from_str(&json).unwrap_or_else(|_| JsonValue::Null);

                    // Get live_logs if available
                    if let Some(live_logs) = progress.get("live_logs").and_then(|v| v.as_array()) {
                        logs.extend(live_logs.iter().cloned());
                    }

                    // Get current test logs if available
                    if let Some(current_test) = progress.get("current_test") {
                        if let Some(test_logs) = current_test.get("logs").and_then(|v| v.as_array())
                        {
                            logs.extend(test_logs.iter().cloned());
                        }
                    }
                } else {
                    return Err(StatusCode::NOT_FOUND);
                }
            }
        }

        // Apply filters
        let filtered_logs: Vec<JsonValue> = logs
            .into_iter()
            .filter(|log| {
                if let Some(file_filter) = &params.file {
                    if let Some(file) = log.get("file").and_then(|v| v.as_str()) {
                        if !file.contains(file_filter) {
                            return false;
                        }
                    }
                }
                true
            })
            .skip(params.offset.unwrap_or(0))
            .take(params.limit.unwrap_or(1000))
            .collect();

        Ok(Json(serde_json::json!({
            "job_id": job_id,
            "logs": filtered_logs,
            "total": filtered_logs.len(),
            "has_more": filtered_logs.len() == params.limit.unwrap_or(1000)
        })))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Get resource usage data
pub async fn get_resource_usage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JsonValue>, StatusCode> {
    let job_id = id.to_string();

    if let Some(redis) = &state.redis_client {
        // Get raw JSON from Redis
        let mut conn = redis
            .client
            .get_tokio_connection_manager()
            .await
            .map_err(|e| {
                tracing::error!("Failed to connect to Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        let key = format!("job:{}:progress", job_id);
        let json_str: Option<String> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get job progress from Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        if let Some(json) = json_str {
            let progress: JsonValue =
                serde_json::from_str(&json).unwrap_or_else(|_| JsonValue::Null);

            let mut resource_data = serde_json::json!({
                "job_id": job_id,
                "current_test_resources": null,
                "test_history": []
            });

            // Get current test resource usage
            if let Some(current_test) = progress.get("current_test") {
                if let Some(resource_usage) = current_test.get("resource_usage") {
                    resource_data["current_test_resources"] = resource_usage.clone();
                }
            }

            // Get historical resource usage from test results
            if let Some(results) = progress
                .get("results")
                .and_then(|r| r.get("results"))
                .and_then(|r| r.as_array())
            {
                let history: Vec<JsonValue> = results
                    .iter()
                    .filter_map(|result| {
                        if let (Some(task_id), Some(metrics)) =
                            (result.get("task_id"), result.get("metrics"))
                        {
                            Some(serde_json::json!({
                                "task_id": task_id,
                                "metrics": metrics
                            }))
                        } else {
                            None
                        }
                    })
                    .collect();

                resource_data["test_history"] = serde_json::Value::Array(history);
            }

            Ok(Json(resource_data))
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Query parameters for log streaming
#[derive(Debug, Deserialize)]
pub struct LogStreamParams {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub file: Option<String>,
}
