//! Job lifecycle operations (complete, fail)

use crate::{rows::JobRow, service::SchedulerService, types::TestResultData};
use anyhow::Result;
use chrono::Utc;
use platform_api_models::*;
use tracing::info;
use uuid::Uuid;

impl SchedulerService {
    /// Mark a job as completed with results
    pub async fn complete_job(&self, job_id: Uuid, result: SubmitResultRequest) -> Result<()> {
        if let Some(pool) = &self.database_pool {
            let now = Utc::now();

            // Extract progress metrics from result
            let result_json = serde_json::to_value(&result.result)?;
            let progress_percent = result_json
                .get("progress")
                .and_then(|p| p.get("progress_percent"))
                .and_then(|v| v.as_f64())
                .map(|v| v * 100.0);
            let total_tasks = result_json
                .get("progress")
                .and_then(|p| p.get("total_tasks"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
            let completed_tasks = result_json
                .get("progress")
                .and_then(|p| p.get("completed_tasks"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
            let resolved_tasks = result_json
                .get("progress")
                .and_then(|p| p.get("resolved_tasks"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
            let unresolved_tasks = result_json
                .get("progress")
                .and_then(|p| p.get("unresolved_tasks"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);

            // Get challenge_id from job
            let job_row =
                sqlx::query_as::<_, JobRow>("SELECT challenge_id FROM jobs WHERE id = $1")
                    .bind(job_id)
                    .fetch_optional(pool.as_ref())
                    .await?;

            let challenge_id = job_row
                .map(|r| r.challenge_id)
                .ok_or_else(|| anyhow::anyhow!("Job {} not found", job_id))?;

            // Update job with progress metrics
            sqlx::query(
                r#"
                UPDATE jobs 
                SET status = 'completed',
                    started_at = COALESCE(started_at, $1),
                    completed_at = $1,
                    result = $2,
                    progress_percent = $4,
                    total_tasks = $5,
                    completed_tasks = $6,
                    resolved_tasks = $7,
                    unresolved_tasks = $8
                WHERE id = $3
                "#,
            )
            .bind(now)
            .bind(&result_json)
            .bind(job_id)
            .bind(progress_percent)
            .bind(total_tasks)
            .bind(completed_tasks)
            .bind(resolved_tasks)
            .bind(unresolved_tasks)
            .execute(pool.as_ref())
            .await?;

            // Extract and store individual test results
            if let Some(results_array) = result_json
                .get("results")
                .and_then(|r| r.get("results"))
                .and_then(|r| r.as_array())
            {
                for test_result in results_array {
                    if let Ok(test_data) = Self::extract_test_result(test_result) {
                        sqlx::query(
                            r#"
                            INSERT INTO job_test_results (
                                job_id, challenge_id, task_id, test_name, status,
                                is_resolved, error_message, execution_time_ms,
                                output_text, logs, metrics
                            )
                            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                            "#,
                        )
                        .bind(job_id)
                        .bind(challenge_id)
                        .bind(&test_data.task_id)
                        .bind(test_data.test_name.as_deref())
                        .bind(&test_data.status)
                        .bind(test_data.is_resolved)
                        .bind(test_data.error_message.as_deref())
                        .bind(test_data.execution_time_ms)
                        .bind(test_data.output_text.as_deref())
                        .bind(&test_data.logs)
                        .bind(&test_data.metrics)
                        .execute(pool.as_ref())
                        .await?;
                    }
                }
            }

            info!(job_id = %job_id, "Job completed with detailed results stored");
        } else {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(&job_id) {
                job.status = JobStatus::Completed;
                job.completed_at = Some(Utc::now());
                if job.started_at.is_none() {
                    job.started_at = Some(Utc::now());
                }
            }
        }

        Ok(())
    }

    /// Extract test result data from Terminal-Bench result JSON
    fn extract_test_result(test_result: &serde_json::Value) -> Result<TestResultData> {
        let task_id = test_result
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing task_id"))?
            .to_string();

        let test_name = test_result
            .get("test_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let is_resolved = test_result
            .get("is_resolved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let status = if is_resolved {
            "passed".to_string()
        } else if test_result.get("error").is_some() {
            "error".to_string()
        } else {
            "failed".to_string()
        };

        let error_message = test_result
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let execution_time_ms = test_result
            .get("execution_time_ms")
            .or_else(|| test_result.get("execution_time"))
            .and_then(|v| v.as_i64());

        let output_text = test_result
            .get("output")
            .or_else(|| test_result.get("output_text"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let logs = test_result
            .get("logs")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let metrics = test_result
            .get("metrics")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        Ok(TestResultData {
            task_id,
            test_name,
            status,
            is_resolved,
            error_message,
            execution_time_ms,
            output_text,
            logs,
            metrics,
        })
    }

    /// Mark a job as failed
    pub async fn fail_job(&self, job_id: Uuid, request: FailJobRequest) -> Result<()> {
        if let Some(pool) = &self.database_pool {
            let now = Utc::now();

            sqlx::query(
                r#"
                UPDATE jobs 
                SET status = 'failed',
                    error_message = $1,
                    completed_at = $2
                WHERE id = $3
                "#,
            )
            .bind(&request.reason)
            .bind(now)
            .bind(job_id)
            .execute(pool.as_ref())
            .await?;

            info!(job_id = %job_id, reason = %request.reason, "Job failed");
        } else {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(&job_id) {
                job.status = JobStatus::Failed;
                job.completed_at = Some(Utc::now());
            }
        }

        Ok(())
    }
}

