use super::{Digest, Hotkey, Id, RuntimeType, Score};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Job status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Pending,
    Claimed,
    Running,
    Completed,
    Failed,
    Timeout,
}

/// Job priority
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobPriority {
    Low,
    Normal,
    High,
    Critical,
}

/// Job metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMetadata {
    pub id: Id,
    pub challenge_id: Id,
    pub validator_hotkey: Option<Hotkey>,
    pub status: JobStatus,
    pub priority: JobPriority,
    pub runtime: RuntimeType,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub timeout_at: Option<DateTime<Utc>>,
    pub retry_count: u32,
    pub max_retries: u32,
}

/// Job claim request
#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimJobRequest {
    pub validator_hotkey: Hotkey,
    pub runtime: RuntimeType,
    pub capabilities: Vec<String>,
}

/// Job claim response
#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimJobResponse {
    pub job: JobMetadata,
    pub harness: HarnessBundle,
    pub datasets: Vec<DatasetArtifact>,
    pub config: JobConfig,
}

/// Job configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobConfig {
    pub timeout: u64,
    pub resources: ResourceLimits,
    pub environment: std::collections::BTreeMap<String, String>,
    pub attestation_required: bool,
    pub policy: Option<String>,
}

/// Submission bundle for evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionBundle {
    pub digest: Digest,
    pub size: u64,
    pub encrypted: bool,
    pub public_key: Option<String>,
    pub metadata: SubmissionMetadata,
}

/// Submission metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionMetadata {
    pub miner_hotkey: Hotkey,
    pub challenge_id: Id,
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

/// Evaluation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub job_id: Id,
    pub submission_id: Id,
    pub scores: std::collections::BTreeMap<String, Score>,
    pub metrics: std::collections::BTreeMap<String, f64>,
    pub logs: Vec<String>,
    pub error: Option<String>,
    pub execution_time: u64,
    pub resource_usage: ResourceUsage,
    pub attestation_receipt: Option<String>,
}

/// Resource usage during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_time: u64,
    pub memory_peak: u64,
    pub disk_usage: u64,
    pub network_bytes: u64,
}

/// Job result submission
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitResultRequest {
    pub job_id: Id,
    pub result: EvalResult,
    pub receipts: Vec<String>,
}

/// Request to fail a job
#[derive(Debug, Serialize, Deserialize)]
pub struct FailJobRequest {
    pub reason: String,
    pub error_details: Option<String>,
}

/// Job list response
#[derive(Debug, Serialize, Deserialize)]
pub struct JobListResponse {
    pub jobs: Vec<JobMetadata>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

/// Job statistics
#[derive(Debug, Serialize, Deserialize)]
pub struct JobStats {
    pub total_jobs: u64,
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub avg_execution_time: f64,
    pub success_rate: f64,
}

use super::challenge::{DatasetArtifact, HarnessBundle, ResourceLimits};
