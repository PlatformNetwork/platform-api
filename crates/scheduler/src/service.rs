//! Scheduler service implementation

use crate::types::SchedulerConfig;
use anyhow::Result;
use platform_api_models::JobMetadata;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Scheduler service for managing job lifecycle
pub struct SchedulerService {
    pub(crate) config: SchedulerConfig,
    pub(crate) database_pool: Option<Arc<PgPool>>,
    // Fallback to in-memory if no database pool
    pub(crate) jobs: tokio::sync::RwLock<std::collections::HashMap<Uuid, JobMetadata>>,
}

impl SchedulerService {
    /// Create a new scheduler service with in-memory storage
    pub fn new(config: &SchedulerConfig) -> Result<Self> {
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
}
