use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Job status for tracking job execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Pending,
    Distributing,
    Running,
    Completed,
    Failed,
}

/// Cache entry for a job being distributed to validators
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobCache {
    pub job_id: String,
    pub challenge_id: String,
    pub compose_hash: String,
    pub status: JobStatus,
    pub assigned_validators: Vec<String>, // validator_hotkeys
    pub challenge_cvm_ws_url: Option<String>, // For forwarding results back to challenge CVM
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl JobCache {
    pub fn new(
        job_id: String,
        challenge_id: String,
        compose_hash: String,
        challenge_cvm_ws_url: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            job_id,
            challenge_id,
            compose_hash,
            status: JobStatus::Pending,
            assigned_validators: Vec::new(),
            challenge_cvm_ws_url,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn mark_distributing(&mut self) {
        self.status = JobStatus::Distributing;
        self.updated_at = Utc::now();
    }

    pub fn mark_running(&mut self, validator_hotkey: String) {
        self.status = JobStatus::Running;
        if !self.assigned_validators.contains(&validator_hotkey) {
            self.assigned_validators.push(validator_hotkey);
        }
        self.updated_at = Utc::now();
    }

    pub fn mark_completed(&mut self) {
        self.status = JobStatus::Completed;
        self.updated_at = Utc::now();
    }

    pub fn mark_failed(&mut self) {
        self.status = JobStatus::Failed;
        self.updated_at = Utc::now();
    }
}
