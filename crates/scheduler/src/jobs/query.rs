//! Job query operations

use crate::{rows::JobRow, service::SchedulerService};
use anyhow::Result;
use platform_api_models::*;
use tracing::info;
use uuid::Uuid;

impl SchedulerService {
    /// List jobs with pagination and optional filters
    pub async fn list_jobs(
        &self,
        page: u32,
        per_page: u32,
        status: Option<String>,
        challenge_id: Option<Uuid>,
    ) -> Result<JobListResponse> {
        if let Some(pool) = &self.database_pool {
            let offset = (page - 1) * per_page;

            // Build query with optional filters
            let rows = if let Some(challenge_id_filter) = challenge_id {
                if let Some(status_filter) = &status {
                    sqlx::query_as::<_, JobRow>(
                        r#"
                        SELECT id, challenge_id, validator_hotkey, status, priority, runtime,
                               created_at, claimed_at, started_at, completed_at, timeout_at,
                               retry_count, max_retries, payload
                        FROM jobs
                        WHERE status = $1 AND challenge_id = $2
                        ORDER BY created_at DESC
                        LIMIT $3 OFFSET $4
                        "#,
                    )
                    .bind(status_filter)
                    .bind(challenge_id_filter)
                    .bind(per_page as i64)
                    .bind(offset as i64)
                    .fetch_all(pool.as_ref())
                    .await?
                } else {
                    sqlx::query_as::<_, JobRow>(
                        r#"
                        SELECT id, challenge_id, validator_hotkey, status, priority, runtime,
                               created_at, claimed_at, started_at, completed_at, timeout_at,
                               retry_count, max_retries, payload
                        FROM jobs
                        WHERE challenge_id = $1
                        ORDER BY created_at DESC
                        LIMIT $2 OFFSET $3
                        "#,
                    )
                    .bind(challenge_id_filter)
                    .bind(per_page as i64)
                    .bind(offset as i64)
                    .fetch_all(pool.as_ref())
                    .await?
                }
            } else if let Some(status_filter) = &status {
                sqlx::query_as::<_, JobRow>(
                    r#"
                    SELECT id, challenge_id, validator_hotkey, status, priority, runtime,
                           created_at, claimed_at, started_at, completed_at, timeout_at,
                           retry_count, max_retries, payload
                    FROM jobs
                    WHERE status = $1
                    ORDER BY created_at DESC
                    LIMIT $2 OFFSET $3
                    "#,
                )
                .bind(status_filter)
                .bind(per_page as i64)
                .bind(offset as i64)
                .fetch_all(pool.as_ref())
                .await?
            } else {
                sqlx::query_as::<_, JobRow>(
                    r#"
                    SELECT id, challenge_id, validator_hotkey, status, priority, runtime,
                           created_at, claimed_at, started_at, completed_at, timeout_at,
                           retry_count, max_retries, payload
                    FROM jobs
                    ORDER BY created_at DESC
                    LIMIT $1 OFFSET $2
                    "#,
                )
                .bind(per_page as i64)
                .bind(offset as i64)
                .fetch_all(pool.as_ref())
                .await?
            };

            let jobs: Vec<JobMetadata> = rows.into_iter().map(|r| r.into()).collect();

            // Get total count
            let total: i64 = if let Some(challenge_id_filter) = challenge_id {
                if let Some(status_filter) = &status {
                    sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM jobs WHERE status = $1 AND challenge_id = $2",
                    )
                    .bind(status_filter)
                    .bind(challenge_id_filter)
                    .fetch_one(pool.as_ref())
                    .await?
                } else {
                    sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM jobs WHERE challenge_id = $1",
                    )
                    .bind(challenge_id_filter)
                    .fetch_one(pool.as_ref())
                    .await?
                }
            } else if let Some(status_filter) = &status {
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs WHERE status = $1")
                    .bind(status_filter)
                    .fetch_one(pool.as_ref())
                    .await?
            } else {
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs")
                    .fetch_one(pool.as_ref())
                    .await?
            };

            Ok(JobListResponse {
                jobs,
                total: total as u64,
                page,
                per_page,
            })
        } else {
            // Fallback to in-memory
            let jobs = self.jobs.read().await;
            let mut job_list: Vec<JobMetadata> = jobs.values().cloned().collect();

            if let Some(status_filter) = &status {
                job_list.retain(|j| format!("{:?}", j.status).to_lowercase() == *status_filter);
            }

            if let Some(challenge_id_filter) = challenge_id {
                job_list.retain(|j| {
                    j.challenge_id.to_string().parse::<Uuid>().ok() == Some(challenge_id_filter)
                });
            }

            job_list.sort_by(|a, b| b.created_at.cmp(&a.created_at));

            let total = job_list.len() as u64;
            let start = ((page - 1) * per_page) as usize;
            let end = (start + per_page as usize).min(job_list.len());
            let paginated_jobs = job_list[start..end].to_vec();

            Ok(JobListResponse {
                jobs: paginated_jobs,
                total,
                page,
                per_page,
            })
        }
    }

    /// Get a specific job by ID
    pub async fn get_job(&self, id: Uuid) -> Result<JobMetadata> {
        if let Some(pool) = &self.database_pool {
            let row = sqlx::query_as::<_, JobRow>(
                r#"
                SELECT id, challenge_id, validator_hotkey, status, priority, runtime,
                       created_at, claimed_at, started_at, completed_at, timeout_at,
                       retry_count, max_retries, payload
                FROM jobs
                WHERE id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(pool.as_ref())
            .await?;

            row.map(Into::into)
                .ok_or_else(|| anyhow::anyhow!("Job not found"))
        } else {
            let jobs = self.jobs.read().await;
            jobs.get(&id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Job not found"))
        }
    }

    /// Get job statistics
    pub async fn get_job_stats(&self) -> Result<JobStats> {
        if let Some(pool) = &self.database_pool {
            let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
                .fetch_one(pool.as_ref())
                .await?;

            let pending: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'pending'")
                    .fetch_one(pool.as_ref())
                    .await?;

            let running: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'running'")
                    .fetch_one(pool.as_ref())
                    .await?;

            let completed: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'completed'")
                    .fetch_one(pool.as_ref())
                    .await?;

            let failed: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'failed'")
                    .fetch_one(pool.as_ref())
                    .await?;

            Ok(JobStats {
                total_jobs: total as u64,
                pending_jobs: pending as u64,
                running_jobs: running as u64,
                completed_jobs: completed as u64,
                failed_jobs: failed as u64,
                avg_execution_time: 0.0,
                success_rate: if total > 0 {
                    completed as f64 / total as f64
                } else {
                    0.0
                },
            })
        } else {
            let jobs = self.jobs.read().await;
            let total = jobs.len() as u64;
            let pending = jobs
                .values()
                .filter(|j| matches!(j.status, JobStatus::Pending))
                .count() as u64;
            let running = jobs
                .values()
                .filter(|j| matches!(j.status, JobStatus::Running))
                .count() as u64;
            let completed = jobs
                .values()
                .filter(|j| matches!(j.status, JobStatus::Completed))
                .count() as u64;
            let failed = jobs
                .values()
                .filter(|j| matches!(j.status, JobStatus::Failed))
                .count() as u64;

            Ok(JobStats {
                total_jobs: total,
                pending_jobs: pending,
                running_jobs: running,
                completed_jobs: completed,
                failed_jobs: failed,
                avg_execution_time: 0.0,
                success_rate: if total > 0 {
                    completed as f64 / total as f64
                } else {
                    0.0
                },
            })
        }
    }
}

