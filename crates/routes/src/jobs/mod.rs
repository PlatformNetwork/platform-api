mod handlers;
mod types;

use axum::{
    routing::{get, post},
    Router,
};

use platform_api::state::AppState;

pub use handlers::*;
pub use types::*;

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
        .route("/challenge/create-job", post(create_job_from_challenge)) // Legacy route
}

