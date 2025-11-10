use anyhow::{Context, Result};
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{error, info, warn};

/// Redis client for job progress logging
#[derive(Clone)]
pub struct RedisClient {
    pub client: Client,
}

/// Job progress data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobProgress {
    pub job_id: String,
    pub status: String,
    pub progress_percent: f64,
    pub total_tasks: Option<i32>,
    pub completed_tasks: Option<i32>,
    pub resolved_tasks: Option<i32>,
    pub unresolved_tasks: Option<i32>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub error_message: Option<String>,
}

/// Job log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobLogEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub level: String,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl RedisClient {
    /// Create a new Redis client
    pub fn new(redis_url: &str) -> Result<Self> {
        let client = Client::open(redis_url).context("Failed to create Redis client")?;

        info!("Redis client initialized");

        Ok(Self { client })
    }

    /// Get a connection manager for async operations
    async fn get_connection(&self) -> Result<ConnectionManager> {
        let manager = self
            .client
            .get_tokio_connection_manager()
            .await
            .context("Failed to get Redis connection manager")?;
        Ok(manager)
    }

    /// Set job progress in Redis with TTL (24 hours)
    pub async fn set_job_progress(&self, progress: &JobProgress) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let key = format!("job:{}:progress", progress.job_id);

        let json = serde_json::to_string(progress).context("Failed to serialize job progress")?;

        // Set with TTL of 24 hours (86400 seconds)
        let ttl: u64 = 86400;
        conn.set_ex(&key, json, ttl)
            .await
            .context("Failed to set job progress in Redis")?;

        Ok(())
    }

    /// Append a log entry to job logs list
    pub async fn append_job_log(&self, job_id: &str, log_entry: &JobLogEntry) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let key = format!("job:{}:logs", job_id);

        let json = serde_json::to_string(log_entry).context("Failed to serialize job log entry")?;

        // Append to list
        conn.rpush::<_, _, ()>(&key, json)
            .await
            .context("Failed to append job log to Redis")?;

        // Set TTL on the list (24 hours)
        let ttl: i64 = 86400;
        conn.expire::<_, ()>(&key, ttl)
            .await
            .context("Failed to set TTL on job logs key")?;

        Ok(())
    }

    /// Get job progress from Redis
    pub async fn get_job_progress(&self, job_id: &str) -> Result<Option<JobProgress>> {
        let mut conn = self.get_connection().await?;
        let key = format!("job:{}:progress", job_id);

        let json: Option<String> = conn
            .get(&key)
            .await
            .context("Failed to get job progress from Redis")?;

        if let Some(json_str) = json {
            let progress: JobProgress =
                serde_json::from_str(&json_str).context("Failed to deserialize job progress")?;
            Ok(Some(progress))
        } else {
            Ok(None)
        }
    }

    /// Get job logs from Redis
    pub async fn get_job_logs(
        &self,
        job_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<JobLogEntry>> {
        let mut conn = self.get_connection().await?;
        let key = format!("job:{}:logs", job_id);

        let limit = limit.unwrap_or(1000); // Default to 1000 entries

        // Get range of logs (from beginning, up to limit)
        let end_index = (limit.saturating_sub(1)) as isize;
        let json_strings: Vec<String> = conn
            .lrange(&key, 0, end_index)
            .await
            .context("Failed to get job logs from Redis")?;

        let mut logs = Vec::new();
        for json_str in json_strings {
            match serde_json::from_str::<JobLogEntry>(&json_str) {
                Ok(log_entry) => logs.push(log_entry),
                Err(e) => {
                    warn!("Failed to deserialize job log entry: {}", e);
                    // Continue with other entries
                }
            }
        }

        Ok(logs)
    }

    /// Delete job progress and logs (cleanup)
    pub async fn delete_job_data(&self, job_id: &str) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let progress_key = format!("job:{}:progress", job_id);
        let logs_key = format!("job:{}:logs", job_id);

        // Delete both keys
        conn.del::<_, i32>(&progress_key)
            .await
            .context("Failed to delete job progress key")?;
        conn.del::<_, i32>(&logs_key)
            .await
            .context("Failed to delete job logs key")?;

        Ok(())
    }

    /// Test Redis connection
    pub async fn test_connection(&self) -> Result<()> {
        let mut conn = self.get_connection().await?;
        // Test connection by trying to get a non-existent key
        let _: Option<String> = conn
            .get("__test_connection__")
            .await
            .context("Failed to test Redis connection")?;
        Ok(())
    }
}

/// Helper function to create a job progress update
pub fn create_job_progress(
    job_id: String,
    status: String,
    progress_percent: f64,
    total_tasks: Option<i32>,
    completed_tasks: Option<i32>,
    resolved_tasks: Option<i32>,
    unresolved_tasks: Option<i32>,
    error_message: Option<String>,
) -> JobProgress {
    JobProgress {
        job_id,
        status,
        progress_percent,
        total_tasks,
        completed_tasks,
        resolved_tasks,
        unresolved_tasks,
        timestamp: chrono::Utc::now(),
        error_message,
    }
}

/// Helper function to create a job log entry
pub fn create_job_log(
    level: String,
    message: String,
    data: Option<serde_json::Value>,
) -> JobLogEntry {
    JobLogEntry {
        timestamp: chrono::Utc::now(),
        level,
        message,
        data,
    }
}
