//! Job result handling and submission

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{JobMetadata, JobTestResult, SubmitResultRequest};
use serde_json::Value as JsonValue;

/// Submit job results
pub async fn submit_results(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(request): Json<SubmitResultRequest>,
) -> Result<(), StatusCode> {
    // Validate job exists and is in correct state
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {} for result submission: {}", job_id, e);
            StatusCode::NOT_FOUND
        })?;

    if job.status != "running" {
        error!("Cannot submit results for job {} with status: {}", job_id, job.status);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Validate result submission request
    if let Err(e) = validate_submit_result_request(&request).await {
        error!("Invalid result submission request: {}", e);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Process results based on type
    match request.result_type.as_str() {
        "test_results" => {
            process_test_results(job_id, &request.results, &state).await?;
        }
        "final_results" => {
            process_final_results(job_id, &request.results, &request.validator_hotkey, &state).await?;
        }
        "benchmark_results" => {
            process_benchmark_results(job_id, &request.results, &request.validator_hotkey, &state).await?;
        }
        _ => {
            error!("Unknown result type: {}", request.result_type);
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    // Store results in database
    store_job_results(job_id, &request, &state).await?;

    info!("Results submitted for job {} by validator {}", 
          job_id, request.validator_hotkey);

    Ok(())
}

/// Get job progress information
pub async fn get_job_progress(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<JobProgressInfo>, StatusCode> {
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {} for progress: {}", job_id, e);
            StatusCode::NOT_FOUND
        })?;

    let progress = state
        .get_job_progress(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job progress {}: {}", job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(JobProgressInfo {
        job_id,
        status: job.status,
        progress_percentage: progress.percentage,
        current_step: progress.current_step,
        total_steps: progress.total_steps,
        estimated_completion: progress.estimated_completion,
        validator_hotkey: job.validator_hotkey,
        started_at: job.started_at,
        updated_at: job.updated_at,
    }))
}

/// Get job test results
pub async fn get_job_test_results(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    Query(params): Query<TestResultsQuery>,
) -> Result<Json<Vec<JobTestResult>>, StatusCode> {
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {} for test results: {}", job_id, e);
            StatusCode::NOT_FOUND
        })?;

    if job.status != "completed" && job.status != "running" {
        error!("Cannot get test results for job {} with status: {}", job_id, job.status);
        return Err(StatusCode::BAD_REQUEST);
    }

    let test_results = state
        .get_job_test_results(job_id, params.test_type)
        .await
        .map_err(|e| {
            error!("Failed to get test results for job {}: {}", job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(test_results))
}

/// Get current test being executed
pub async fn get_current_test(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<Option<CurrentTestInfo>>, StatusCode> {
    let job = state
        .scheduler
        .get_job(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get job {} for current test: {}", job_id, e);
            StatusCode::NOT_FOUND
        })?;

    if job.status != "running" {
        return Ok(Json(None));
    }

    let current_test = state
        .get_current_test(job_id)
        .await
        .map_err(|e| {
            error!("Failed to get current test for job {}: {}", job_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(current_test))
}

/// Process test results
async fn process_test_results(
    job_id: Uuid,
    results: &JsonValue,
    state: &AppState,
) -> Result<(), anyhow::Error> {
    // Extract test results from JSON
    let test_results: Vec<JobTestResult> = serde_json::from_value(results.clone())?;

    // Validate each test result
    for test_result in &test_results {
        validate_test_result(test_result)?;
    }

    // Update job progress based on test results
    let passed_count = test_results.iter().filter(|r| r.passed).count();
    let total_count = test_results.len();
    let progress_percentage = (passed_count as f64 / total_count as f64) * 100.0;

    state
        .update_job_progress(job_id, progress_percentage, Some("tests_completed"))
        .await?;

    info!("Processed {} test results for job {}", total_count, job_id);

    Ok(())
}

/// Process final results
async fn process_final_results(
    job_id: Uuid,
    results: &JsonValue,
    validator_hotkey: &str,
    state: &AppState,
) -> Result<(), anyhow::Error> {
    // Validate final results format
    if !results.get("summary").is_some() {
        return Err(anyhow::anyhow!("Final results must include a summary"));
    }

    // Complete the job with final results
    state
        .scheduler
        .complete_job(job_id, validator_hotkey.to_string(), results.clone())
        .await?;

    info!("Processed final results for job {}", job_id);

    Ok(())
}

/// Process benchmark results
async fn process_benchmark_results(
    job_id: Uuid,
    results: &JsonValue,
    validator_hotkey: &str,
    state: &AppState,
) -> Result<(), anyhow::Error> {
    // Extract benchmark metrics
    let benchmark_metrics = extract_benchmark_metrics(results)?;

    // Store benchmark metrics
    state
        .store_benchmark_metrics(job_id, validator_hotkey, benchmark_metrics)
        .await?;

    info!("Processed benchmark results for job {}", job_id);

    Ok(())
}

/// Store job results in database
async fn store_job_results(
    job_id: Uuid,
    request: &SubmitResultRequest,
    state: &AppState,
) -> Result<(), anyhow::Error> {
    let result_record = serde_json::json!({
        "job_id": job_id,
        "validator_hotkey": request.validator_hotkey,
        "result_type": request.result_type,
        "results": request.results,
        "submitted_at": chrono::Utc::now(),
    });

    sqlx::query!(
        r#"
        INSERT INTO job_results (job_id, validator_hotkey, result_type, results, submitted_at)
        VALUES ($1, $2, $3, $4, $5)
        "#,
        job_id,
        request.validator_hotkey,
        request.result_type,
        request.results,
        chrono::Utc::now()
    )
    .execute(&state.db_pool)
    .await?;

    Ok(())
}

/// Validate test result
fn validate_test_result(test_result: &JobTestResult) -> Result<(), anyhow::Error> {
    if test_result.test_name.is_empty() {
        return Err(anyhow::anyhow!("Test name cannot be empty"));
    }

    if test_result.execution_time.is_negative() {
        return Err(anyhow::anyhow!("Execution time cannot be negative"));
    }

    Ok(())
}

/// Extract benchmark metrics from results
fn extract_benchmark_metrics(results: &JsonValue) -> Result<BenchmarkMetrics, anyhow::Error> {
    let metrics: BenchmarkMetrics = serde_json::from_value(results.clone())
        .map_err(|e| anyhow::anyhow!("Invalid benchmark metrics format: {}", e))?;

    if metrics.cpu_usage.is_none() && metrics.memory_usage.is_none() && metrics.execution_time.is_none() {
        return Err(anyhow::anyhow!("At least one metric must be provided"));
    }

    Ok(metrics)
}

// Request/Response types
#[derive(Deserialize)]
pub struct TestResultsQuery {
    pub test_type: Option<String>,
}

#[derive(serde::Serialize)]
pub struct JobProgressInfo {
    pub job_id: Uuid,
    pub status: String,
    pub progress_percentage: f64,
    pub current_step: Option<String>,
    pub total_steps: Option<i32>,
    pub estimated_completion: Option<chrono::DateTime<chrono::Utc>>,
    pub validator_hotkey: Option<String>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(serde::Serialize)]
pub struct CurrentTestInfo {
    pub test_name: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub estimated_duration: Option<i32>, // in seconds
    pub progress_percentage: f64,
}

#[derive(serde::Deserialize, Debug)]
pub struct BenchmarkMetrics {
    pub cpu_usage: Option<f64>,
    pub memory_usage: Option<u64>,
    pub execution_time: Option<i64>,
    pub disk_io: Option<DiskIOMetrics>,
    pub network_io: Option<NetworkIOMetrics>,
}

#[derive(serde::Deserialize, Debug)]
pub struct DiskIOMetrics {
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub read_ops: u64,
    pub write_ops: u64,
}

#[derive(serde::Deserialize, Debug)]
pub struct NetworkIOMetrics {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
}
