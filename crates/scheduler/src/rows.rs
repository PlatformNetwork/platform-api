//! Database row structures for scheduler

use chrono::{DateTime, Utc};
use platform_api_models::*;
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;

/// Database row for jobs table
#[derive(Debug, FromRow)]
pub struct JobRow {
    pub id: Uuid,
    pub challenge_id: Uuid,
    pub validator_hotkey: Option<String>,
    pub status: String,
    pub priority: String,
    pub runtime: String,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub timeout_at: Option<DateTime<Utc>>,
    pub retry_count: i32,
    pub max_retries: i32,
    pub payload: Option<JsonValue>,
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
            validator_hotkey: row.validator_hotkey.map(Hotkey::from),
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
