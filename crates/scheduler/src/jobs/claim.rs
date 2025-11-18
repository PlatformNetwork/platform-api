//! Job claim operations

use crate::{rows::JobRow, service::SchedulerService};
use anyhow::Result;
use chrono::Utc;
use platform_api_models::*;
use std::collections::BTreeMap;
use tracing::info;
use uuid::Uuid;

impl SchedulerService {
    /// Claim the next available pending job
    pub async fn claim_job(&self, request: ClaimJobRequest) -> Result<ClaimJobResponse> {
        if let Some(pool) = &self.database_pool {
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

                info!(job_id = %job.id, validator_hotkey = %request.validator_hotkey, "Claimed job");

                Ok(ClaimJobResponse {
                    job,
                    harness: HarnessBundle {
                        digest: Digest::from(""),
                        size: 0,
                        image_ref: None,
                        manifest: None,
                        config: HarnessConfig::default(),
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
                    config: HarnessConfig::default(),
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

    /// Claim a specific job by ID
    pub async fn claim_specific_job(
        &self,
        job_id: Uuid,
        request: ClaimJobRequest,
    ) -> Result<ClaimJobResponse> {
        if let Some(pool) = &self.database_pool {
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
                    config: HarnessConfig::default(),
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
                    config: HarnessConfig::default(),
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

    /// Get next available job for validator (uses claim_job internally)
    pub async fn get_next_job(
        &self,
        validator_hotkey: String,
        runtime: Option<String>,
    ) -> Result<Option<ClaimJobResponse>> {
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
}

