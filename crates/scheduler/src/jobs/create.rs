//! Job creation operations

use crate::{rows::JobRow, service::SchedulerService, types::CreateJobRequest};
use anyhow::Result;
use chrono::Utc;
use platform_api_models::*;
use tracing::{info, warn};
use uuid::Uuid;

impl SchedulerService {
    /// Create a new job
    pub async fn create_job(&self, request: CreateJobRequest) -> Result<JobMetadata> {
        let job_id = Uuid::new_v4();
        let now = Utc::now();

        // Convert challenge_id to Uuid if it's a string
        let challenge_uuid = match request.challenge_id.to_string().parse::<Uuid>() {
            Ok(uuid) => uuid,
            Err(_) => {
                warn!(
                    "challenge_id '{}' is not a valid UUID, using default",
                    request.challenge_id.to_string()
                );
                Uuid::new_v4()
            }
        };

        let challenge_uuid_for_db = challenge_uuid;

        let job = JobMetadata {
            id: Id::from(job_id),
            challenge_id: Id::from(challenge_uuid),
            validator_hotkey: None,
            status: JobStatus::Pending,
            priority: request.priority.unwrap_or(JobPriority::Normal),
            runtime: request.runtime.clone(),
            created_at: now,
            claimed_at: None,
            started_at: None,
            completed_at: None,
            timeout_at: request
                .timeout
                .map(|secs| now + chrono::Duration::seconds(secs as i64)),
            retry_count: 0,
            max_retries: request.max_retries.unwrap_or(3),
            payload: Some(request.payload.clone()),
        };

        if let Some(pool) = &self.database_pool {
            let status_str = match job.status {
                JobStatus::Pending => "pending",
                JobStatus::Claimed => "claimed",
                JobStatus::Running => "running",
                JobStatus::Completed => "completed",
                JobStatus::Failed => "failed",
                JobStatus::Timeout => "timeout",
            };

            let priority_str = match job.priority {
                JobPriority::Low => "low",
                JobPriority::Normal => "normal",
                JobPriority::High => "high",
                JobPriority::Critical => "critical",
            };

            sqlx::query(
                r#"
                INSERT INTO jobs (
                    id, challenge_id, status, priority, runtime, payload,
                    created_at, timeout_at, retry_count, max_retries
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                "#,
            )
            .bind(job_id)
            .bind(challenge_uuid_for_db)
            .bind(status_str)
            .bind(priority_str)
            .bind(job.runtime.to_string())
            .bind(serde_json::to_value(&request.payload)?)
            .bind(job.created_at)
            .bind(job.timeout_at)
            .bind(job.retry_count as i32)
            .bind(job.max_retries as i32)
            .execute(pool.as_ref())
            .await?;

            info!(job_id = %job_id, challenge_id = %job.challenge_id, "Created job in database");
        } else {
            let mut jobs = self.jobs.write().await;
            jobs.insert(job_id, job.clone());
            info!(job_id = %job_id, "Created job in memory");
        }

        Ok(job)
    }
}

