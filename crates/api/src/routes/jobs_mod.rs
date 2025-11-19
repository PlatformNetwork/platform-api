//! Refactored jobs module with organized components

pub mod job_management;
pub mod job_results;
pub mod job_monitoring;

use axum::Router;
use crate::state::AppState;

pub use job_management::{
    create_job, list_jobs, get_job, claim_job, claim_specific_job,
    complete_job, fail_job, get_next_job, get_job_stats, get_pending_jobs,
    ListJobsQuery, GetNextJobQuery, JobStatsQuery, PendingJobsQuery,
    CompleteJobRequest, FailJobRequest,
};

pub use job_results::{
    submit_results, get_job_progress, get_job_test_results, get_current_test,
    TestResultsQuery, JobProgressInfo, CurrentTestInfo,
    BenchmarkMetrics, DiskIOMetrics, NetworkIOMetrics,
};

pub use job_monitoring::{
    stream_logs, get_resource_usage, get_job_metrics, get_job_status_stream,
    get_job_timeline, LogStreamQuery, ResourceUsageQuery,
    ResourceUsageInfo, CpuUsagePoint, MemoryUsagePoint, DiskIOPoint, NetworkIOPoint,
    JobMetrics, TimelineEvent,
};

/// Create jobs router with all modular routes
pub fn create_jobs_router() -> Router<AppState> {
    super::jobs_refactored::create_router()
}

/// Initialize all job-related services
pub async fn initialize_job_services(state: AppState) {
    super::jobs_refactored::initialize_job_services(state).await;
}

/// Get job service statistics
pub async fn get_job_service_stats(state: &AppState) -> serde_json::Value {
    super::jobs_refactored::get_job_service_stats(state).await
}

/// Update job service configuration
pub async fn update_job_config(
    state: &AppState,
    config: serde_json::Value,
) -> Result<(), anyhow::Error> {
    super::jobs_refactored::update_job_config(state, config).await
}
