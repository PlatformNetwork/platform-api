use serde::Deserialize;
use serde_json::Value as JsonValue;
use uuid::Uuid;

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
#[derive(Debug, Clone, Deserialize)]
pub struct FailJobRequest {
    pub reason: String,
    pub error_details: Option<String>,
}

/// Query parameters for pending jobs
#[derive(Debug, Deserialize)]
pub struct PendingJobsParams {
    pub validator_hotkey: Option<String>,
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

/// Query parameters for log streaming
#[derive(Debug, Deserialize)]
pub struct LogStreamParams {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub file: Option<String>,
}

