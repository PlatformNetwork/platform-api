//! Type definitions for scheduler requests and responses

use platform_api_models::*;
use serde_json::Value as JsonValue;

/// Request to create a new job
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateJobRequest {
    pub challenge_id: Id,
    pub payload: JsonValue,
    pub priority: Option<JobPriority>,
    pub runtime: RuntimeType,
    pub timeout: Option<u64>,
    pub max_retries: Option<u32>,
}

/// Scheduler configuration
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub max_concurrent_jobs: u32,
    pub job_timeout: u64,
    pub retry_attempts: u32,
    pub retry_delay: u64,
    pub cleanup_interval: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_jobs: 100,
            job_timeout: 3600,
            retry_attempts: 3,
            retry_delay: 60,
            cleanup_interval: 3600,
        }
    }
}

/// Test result data structure for storing individual test outcomes
#[derive(Debug, Clone)]
pub(crate) struct TestResultData {
    pub task_id: String,
    pub test_name: Option<String>,
    pub status: String,
    pub is_resolved: bool,
    pub error_message: Option<String>,
    pub execution_time_ms: Option<i64>,
    pub output_text: Option<String>,
    pub logs: JsonValue,
    pub metrics: JsonValue,
}
