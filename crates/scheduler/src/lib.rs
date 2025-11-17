use anyhow::Result;
use chrono::{DateTime, Utc};
use platform_api_models::*;
use serde_json::Value as JsonValue;
use sqlx::{FromRow, PgPool};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

mod capacity;
pub use capacity::*;

mod scoring;
pub use scoring::*;

/// Database row for jobs table
#[derive(Debug, FromRow)]
struct JobRow {
    id: Uuid,
    challenge_id: Uuid, // Changed from String to Uuid to match database schema
    validator_hotkey: Option<String>,
    status: String,
    priority: String,
    runtime: String,
    created_at: DateTime<Utc>,
    claimed_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    timeout_at: Option<DateTime<Utc>>,
    retry_count: i32,
    max_retries: i32,
    payload: Option<JsonValue>,
}

impl From<JobRow> for JobMetadata {
    fn from(row: JobRow) -> Self {
        let status = match row.status.as_str() {
            "pending" => JobStatus::Pending,
            "claimed" => JobStatus::Claimed,
            "running" => JobStatus::Running,
            "completed" => JobStatus::Completed,
            "failed" => JobStatus::Failed,
            "timeout" => JobStatus::Timeout,
            _ => JobStatus::Pending,
        };

        let priority = match row.priority.as_str() {
            "low" => JobPriority::Low,
            "normal" => JobPriority::Normal,
            "high" => JobPriority::High,
            "critical" => JobPriority::Critical,
            _ => JobPriority::Normal,
        };

        let runtime = RuntimeType::from(row.runtime.as_str());

        JobMetadata {
            id: Id::from(row.id),
            challenge_id: Id::from(row.challenge_id),
            validator_hotkey: row.validator_hotkey.map(|h| Hotkey::from(h)),
            status,
            priority,
            runtime,
            created_at: row.created_at,
            claimed_at: row.claimed_at,
            started_at: row.started_at,
            completed_at: row.completed_at,
            timeout_at: row.timeout_at,
            retry_count: row.retry_count as u32,
            max_retries: row.max_retries as u32,
            payload: row.payload,
        }
    }
}

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

/// Scheduler service
pub struct SchedulerService {
    config: SchedulerConfig,
    database_pool: Option<Arc<PgPool>>,
    // Fallback to in-memory if no database pool
    jobs: tokio::sync::RwLock<std::collections::HashMap<Uuid, JobMetadata>>,
}

impl SchedulerService {
    pub fn new(config: &SchedulerConfig) -> std::result::Result<Self, anyhow::Error> {
        Ok(Self {
            config: config.clone(),
            database_pool: None,
            jobs: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        })
    }

    /// Create scheduler with database pool (for PostgreSQL storage)
    pub fn with_database(config: &SchedulerConfig, database_pool: Arc<PgPool>) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            database_pool: Some(database_pool),
            jobs: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        })
    }

    /// Create a new job
    pub async fn create_job(&self, request: CreateJobRequest) -> Result<JobMetadata> {
        let job_id = Uuid::new_v4();
        let now = Utc::now();

        // Convert challenge_id to Uuid if it's a string
        let challenge_uuid = match request.challenge_id.to_string().parse::<Uuid>() {
            Ok(uuid) => uuid,
            Err(_) => {
                // If challenge_id is not a UUID, try to use it as-is or generate a default
                // For now, we'll use a default UUID - in production this should be resolved properly
                warn!(
                    "challenge_id '{}' is not a valid UUID, using default",
                    request.challenge_id.to_string()
                );
                Uuid::new_v4()
            }
        };

        // Store challenge_uuid for database insertion
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
            // Store in PostgreSQL
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

            // Insert job into database
            // Pass UUID directly - sqlx automatically handles PostgreSQL UUID conversion
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
            .bind(challenge_uuid_for_db) // sqlx automatically converts uuid::Uuid to PostgreSQL UUID type
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
            // Fallback to in-memory
            let mut jobs = self.jobs.write().await;
            jobs.insert(job_id, job.clone());
            info!(job_id = %job_id, "Created job in memory");
        }

        Ok(job)
    }

    pub async fn list_jobs(
        &self,
        page: u32,
        per_page: u32,
        status: Option<String>,
        challenge_id: Option<Uuid>,
    ) -> Result<JobListResponse> {
        if let Some(pool) = &self.database_pool {
            // Query from PostgreSQL
            let offset = (page - 1) * per_page;

            // Build query with optional filters using query_as
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

            // Convert rows to JobMetadata
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
                    .await
                    .unwrap_or(0)
                } else {
                    sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM jobs WHERE challenge_id = $1",
                    )
                    .bind(challenge_id_filter)
                    .fetch_one(pool.as_ref())
                    .await
                    .unwrap_or(0)
                }
            } else if let Some(status_filter) = &status {
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs WHERE status = $1")
                    .bind(status_filter)
                    .fetch_one(pool.as_ref())
                    .await
                    .unwrap_or(0)
            } else {
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs")
                    .fetch_one(pool.as_ref())
                    .await
                    .unwrap_or(0)
            };

            Ok(JobListResponse {
                jobs,
                total: total as u64,
                page,
                per_page,
            })
        } else {
            // Fallback to in-memory
            let jobs_map = self.jobs.read().await;
            let mut jobs: Vec<JobMetadata> = jobs_map.values().cloned().collect();

            // Filter by status
            if let Some(status_filter) = &status {
                let status_enum = match status_filter.as_str() {
                    "pending" => JobStatus::Pending,
                    "claimed" => JobStatus::Claimed,
                    "running" => JobStatus::Running,
                    "completed" => JobStatus::Completed,
                    "failed" => JobStatus::Failed,
                    "timeout" => JobStatus::Timeout,
                    _ => {
                        return Ok(JobListResponse {
                            jobs: vec![],
                            total: 0,
                            page,
                            per_page,
                        })
                    }
                };
                jobs.retain(|j| j.status == status_enum);
            }

            // Filter by challenge_id
            if let Some(challenge_id_filter) = challenge_id {
                jobs.retain(|j| j.challenge_id.to_string() == challenge_id_filter.to_string());
            }

            let total = jobs.len() as u64;
            let offset = ((page - 1) * per_page) as usize;
            let end = (offset + per_page as usize).min(jobs.len());
            jobs = jobs[offset..end].to_vec();

            Ok(JobListResponse {
                jobs,
                total,
                page,
                per_page,
            })
        }
    }

    pub async fn get_job(&self, id: Uuid) -> Result<JobMetadata> {
        if let Some(pool) = &self.database_pool {
            let row = sqlx::query_as::<_, JobRow>(
                r#"
                SELECT id, challenge_id, validator_hotkey, status, priority, runtime,
                       created_at, claimed_at, started_at, completed_at, timeout_at,
                       retry_count, max_retries, payload
                FROM jobs WHERE id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(pool.as_ref())
            .await?;

            row.map(|r| r.into())
                .ok_or_else(|| anyhow::anyhow!("Job not found: {}", id))
        } else {
            let jobs = self.jobs.read().await;
            jobs.get(&id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Job not found: {}", id))
        }
    }

    pub async fn claim_job(&self, request: ClaimJobRequest) -> Result<ClaimJobResponse> {
        if let Some(pool) = &self.database_pool {
            // Find and claim a pending job
            let now = Utc::now();

            // Try to claim a pending job (atomic update)
            let row = sqlx::query_as::<_, JobRow>(
                r#"
                UPDATE jobs 
                SET status = 'claimed',
                    validator_hotkey = $1,
                    claimed_at = $2
                WHERE id = (
                    SELECT id FROM jobs 
                    WHERE status = 'pending' 
                    ORDER BY created_at ASC 
                    LIMIT 1
                    FOR UPDATE SKIP LOCKED
                )
                RETURNING id, challenge_id, validator_hotkey, status, priority, runtime,
                          created_at, claimed_at, started_at, completed_at, timeout_at,
                          retry_count, max_retries, payload
                "#,
            )
            .bind(request.validator_hotkey.to_string())
            .bind(now)
            .fetch_optional(pool.as_ref())
            .await?;

            if let Some(r) = row {
                let job: JobMetadata = r.into();

                // Payload is already stored in the job (we can load it later if needed)

                info!(job_id = %job.id, validator_hotkey = %request.validator_hotkey, "Claimed job");

                // Create response with minimal config (for now)
                Ok(ClaimJobResponse {
                    job,
                    harness: HarnessBundle {
                        digest: Digest::from(""),
                        size: 0,
                        image_ref: None,
                        manifest: None,
                        config: platform_api_models::HarnessConfig::default(),
                    },
                    datasets: vec![],
                    config: JobConfig {
                        timeout: self.config.job_timeout,
                        resources: ResourceLimits {
                            cpu_cores: 1,
                            memory_mb: 1024,
                            disk_mb: 10240,
                            network_enabled: true,
                        },
                        environment: BTreeMap::new(),
                        attestation_required: false,
                        policy: None,
                    },
                })
            } else {
                Err(anyhow::anyhow!("No pending jobs available"))
            }
        } else {
            // Fallback to in-memory
            let mut jobs = self.jobs.write().await;
            let job = jobs
                .values_mut()
                .find(|j| j.status == JobStatus::Pending)
                .ok_or_else(|| anyhow::anyhow!("No pending jobs available"))?;

            job.status = JobStatus::Claimed;
            job.validator_hotkey = Some(request.validator_hotkey.clone());
            job.claimed_at = Some(Utc::now());

            Ok(ClaimJobResponse {
                job: job.clone(),
                harness: HarnessBundle {
                    digest: Digest::from(""),
                    size: 0,
                    image_ref: None,
                    manifest: None,
                    config: platform_api_models::HarnessConfig::default(),
                },
                datasets: vec![],
                config: JobConfig {
                    timeout: self.config.job_timeout,
                    resources: ResourceLimits {
                        cpu_cores: 1,
                        memory_mb: 1024,
                        disk_mb: 10240,
                        network_enabled: true,
                    },
                    environment: BTreeMap::new(),
                    attestation_required: false,
                    policy: None,
                },
            })
        }
    }

    pub async fn claim_specific_job(
        &self,
        job_id: Uuid,
        request: ClaimJobRequest,
    ) -> Result<ClaimJobResponse> {
        if let Some(pool) = &self.database_pool {
            // Atomically claim the specific job
            let now = Utc::now();

            let row = sqlx::query_as::<_, JobRow>(
                r#"
                UPDATE jobs 
                SET status = 'claimed',
                    validator_hotkey = $1,
                    claimed_at = $2
                WHERE id = $3 AND status = 'pending'
                RETURNING id, challenge_id, validator_hotkey, status, priority, runtime,
                          created_at, claimed_at, started_at, completed_at, timeout_at,
                          retry_count, max_retries, payload
                "#,
            )
            .bind(request.validator_hotkey.to_string())
            .bind(now)
            .bind(job_id)
            .fetch_optional(pool.as_ref())
            .await?;

            let r = row.ok_or_else(|| anyhow::anyhow!("Job not available or already claimed"))?;
            let job: JobMetadata = r.into();

            info!(job_id = %job.id, validator_hotkey = %request.validator_hotkey, "Claimed specific job");

            Ok(ClaimJobResponse {
                job,
                harness: HarnessBundle {
                    digest: Digest::from(""),
                    size: 0,
                    image_ref: None,
                    manifest: None,
                    config: platform_api_models::HarnessConfig::default(),
                },
                datasets: vec![],
                config: JobConfig {
                    timeout: self.config.job_timeout,
                    resources: ResourceLimits {
                        cpu_cores: 1,
                        memory_mb: 1024,
                        disk_mb: 10240,
                        network_enabled: true,
                    },
                    environment: BTreeMap::new(),
                    attestation_required: false,
                    policy: None,
                },
            })
        } else {
            // Fallback to in-memory
            let mut jobs = self.jobs.write().await;
            let job = jobs
                .get_mut(&job_id)
                .ok_or_else(|| anyhow::anyhow!("Job not found"))?;

            if job.status != JobStatus::Pending {
                return Err(anyhow::anyhow!("Job not available or already claimed"));
            }

            job.status = JobStatus::Claimed;
            job.validator_hotkey = Some(request.validator_hotkey.clone());
            job.claimed_at = Some(Utc::now());

            Ok(ClaimJobResponse {
                job: job.clone(),
                harness: HarnessBundle {
                    digest: Digest::from(""),
                    size: 0,
                    image_ref: None,
                    manifest: None,
                    config: platform_api_models::HarnessConfig::default(),
                },
                datasets: vec![],
                config: JobConfig {
                    timeout: self.config.job_timeout,
                    resources: ResourceLimits {
                        cpu_cores: 1,
                        memory_mb: 1024,
                        disk_mb: 10240,
                        network_enabled: true,
                    },
                    environment: BTreeMap::new(),
                    attestation_required: false,
                    policy: None,
                },
            })
        }
    }

    pub async fn complete_job(&self, job_id: Uuid, result: SubmitResultRequest) -> Result<()> {
        if let Some(pool) = &self.database_pool {
            let now = Utc::now();

            // Extract progress metrics from result
            let result_json = serde_json::to_value(&result.result)?;
            let progress_percent = result_json
                .get("progress")
                .and_then(|p| p.get("progress_percent"))
                .and_then(|v| v.as_f64())
                .map(|v| (v * 100.0) as f64);
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
            .and_then(|v| v.as_i64())
            .map(|v| v as i64);

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

    pub async fn get_next_job(
        &self,
        validator_hotkey: String,
        runtime: Option<String>,
    ) -> Result<Option<ClaimJobResponse>> {
        // Use claim_job to get next available job
        let request = ClaimJobRequest {
            validator_hotkey: Hotkey::from(validator_hotkey),
            runtime: runtime
                .map(|r| RuntimeType::from(r.as_str()))
                .unwrap_or(RuntimeType::Docker),
            capabilities: vec![],
        };

        match self.claim_job(request).await {
            Ok(response) => Ok(Some(response)),
            Err(_) => Ok(None),
        }
    }

    pub async fn get_job_stats(&self) -> Result<JobStats> {
        if let Some(pool) = &self.database_pool {
            // Get stats using separate queries to avoid macro issues
            let total: i64 = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs")
                .fetch_one(pool.as_ref())
                .await
                .unwrap_or(0);

            let pending: i64 =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs WHERE status = 'pending'")
                    .fetch_one(pool.as_ref())
                    .await
                    .unwrap_or(0);

            let running: i64 = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM jobs WHERE status IN ('claimed', 'running')",
            )
            .fetch_one(pool.as_ref())
            .await
            .unwrap_or(0);

            let completed: i64 = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM jobs WHERE status = 'completed'",
            )
            .fetch_one(pool.as_ref())
            .await
            .unwrap_or(0);

            let failed: i64 =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs WHERE status = 'failed'")
                    .fetch_one(pool.as_ref())
                    .await
                    .unwrap_or(0);

            // Calculate success rate
            let success_rate = if total > 0 {
                (completed as f64) / (total as f64)
            } else {
                0.0
            };

            // Calculate average execution time
            let avg_time = sqlx::query_scalar::<_, Option<f64>>(
                r#"
                SELECT AVG(EXTRACT(EPOCH FROM (completed_at - started_at)))
                FROM jobs
                WHERE status = 'completed' AND started_at IS NOT NULL AND completed_at IS NOT NULL
                "#,
            )
            .fetch_one(pool.as_ref())
            .await
            .ok()
            .flatten();

            let avg_execution_time = avg_time.unwrap_or(0.0);

            Ok(JobStats {
                total_jobs: total as u64,
                pending_jobs: pending as u64,
                running_jobs: running as u64,
                completed_jobs: completed as u64,
                failed_jobs: failed as u64,
                avg_execution_time,
                success_rate,
            })
        } else {
            // Fallback to in-memory
            let jobs = self.jobs.read().await;
            let total = jobs.len() as u64;
            let pending = jobs
                .values()
                .filter(|j| j.status == JobStatus::Pending)
                .count() as u64;
            let running = jobs
                .values()
                .filter(|j| matches!(j.status, JobStatus::Claimed | JobStatus::Running))
                .count() as u64;
            let completed = jobs
                .values()
                .filter(|j| j.status == JobStatus::Completed)
                .count() as u64;
            let failed = jobs
                .values()
                .filter(|j| j.status == JobStatus::Failed)
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
struct TestResultData {
    task_id: String,
    test_name: Option<String>,
    status: String,
    is_resolved: bool,
    error_message: Option<String>,
    execution_time_ms: Option<i64>,
    output_text: Option<String>,
    logs: JsonValue,
    metrics: JsonValue,
}
