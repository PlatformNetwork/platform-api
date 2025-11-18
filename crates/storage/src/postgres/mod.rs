//! PostgreSQL storage backend implementation

mod challenges;
mod emissions;
mod nodes;
mod pools;
mod rows;

pub use rows::*;

use super::StorageBackend;
use anyhow::Result;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::info;

/// PostgreSQL storage backend
pub struct PostgresStorageBackend {
    pool: PgPool,
}

impl PostgresStorageBackend {
    /// Create a new PostgreSQL storage backend
    pub async fn new(database_url: &str) -> Result<Self> {
        info!("Connecting to PostgreSQL database...");

        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL: {}", e))?;

        info!("✅ Successfully connected to PostgreSQL");

        // Check if migrations table exists to determine if migrations are needed
        let migrations_exist = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                SELECT FROM information_schema.tables 
                WHERE table_schema = 'public' 
                AND table_name = '_sqlx_migrations'
            )",
        )
        .fetch_one(&pool)
        .await
        .unwrap_or(false);

        if migrations_exist {
            info!("Migrations table exists, checking for pending migrations...");
        } else {
            info!("No migrations table found, running all migrations...");
        }

        // Run migrations - sqlx will only apply new ones
        if let Err(e) = sqlx::migrate!("./migrations").run(&pool).await {
            // If it's a duplicate/prepared statement error, it's likely a concurrency issue
            if e.to_string().contains("already exists")
                || e.to_string().contains("duplicate")
                || e.to_string().contains("prepared statement")
            {
                info!("⚠️  Skipping migrations (likely already applied or concurrent execution)");
            } else {
                return Err(anyhow::anyhow!("Failed to run migrations: {}", e));
            }
        } else {
            info!("✅ Database migrations completed");
        }

        Ok(Self { pool })
    }

    /// Get the underlying database connection pool
    pub fn get_db_pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl StorageBackend for PostgresStorageBackend {
    async fn list_challenges(
        &self,
        page: u32,
        per_page: u32,
        status: Option<String>,
        visibility: Option<String>,
    ) -> Result<platform_api_models::ChallengeListResponse> {
        self.list_challenges_impl(page, per_page, status, visibility)
            .await
    }

    async fn get_challenge(
        &self,
        id: uuid::Uuid,
    ) -> Result<platform_api_models::ChallengeDetailResponse> {
        self.get_challenge_impl(id).await
    }

    async fn get_challenge_emissions(
        &self,
        id: uuid::Uuid,
    ) -> Result<platform_api_models::EmissionSchedule> {
        self.get_challenge_emissions_impl(id).await
    }

    async fn get_subnet_config(&self) -> Result<platform_api_models::SubnetConfig> {
        self.get_subnet_config_impl().await
    }

    async fn update_subnet_config(
        &self,
        config: platform_api_models::SubnetConfig,
    ) -> Result<platform_api_models::SubnetConfig> {
        self.update_subnet_config_impl(config).await
    }

    async fn validate_config(
        &self,
        config: &platform_api_models::UpdateConfigRequest,
    ) -> Result<platform_api_models::ConfigValidationResult> {
        self.validate_config_impl(config).await
    }

    async fn create_config_backup(
        &self,
        request: crate::CreateBackupRequest,
    ) -> Result<platform_api_models::ConfigBackup> {
        self.create_config_backup_impl(request).await
    }

    async fn restore_config(
        &self,
        request: platform_api_models::RestoreConfigRequest,
    ) -> Result<()> {
        self.restore_config_impl(request).await
    }

    async fn list_config_backups(&self) -> Result<Vec<platform_api_models::ConfigBackup>> {
        self.list_config_backups_impl().await
    }

    async fn get_config_backup(&self, id: uuid::Uuid) -> Result<platform_api_models::ConfigBackup> {
        self.get_config_backup_impl(id).await
    }

    async fn get_config_history(&self) -> Result<Vec<platform_api_models::ConfigChangeLog>> {
        self.get_config_history_impl().await
    }

    async fn list_emission_schedules(
        &self,
        status: Option<String>,
        emission_type: Option<String>,
        challenge_id: Option<uuid::Uuid>,
    ) -> Result<Vec<platform_api_models::EmissionSchedule>> {
        self.list_emission_schedules_impl(status, emission_type, challenge_id)
            .await
    }

    async fn get_emission_schedule(
        &self,
        id: uuid::Uuid,
    ) -> Result<platform_api_models::EmissionSchedule> {
        self.get_emission_schedule_impl(id).await
    }

    async fn create_emission_schedule(
        &self,
        request: platform_api_models::CreateEmissionScheduleRequest,
    ) -> Result<platform_api_models::EmissionSchedule> {
        self.create_emission_schedule_impl(request).await
    }

    async fn update_emission_schedule(
        &self,
        id: uuid::Uuid,
        request: platform_api_models::UpdateEmissionScheduleRequest,
    ) -> Result<platform_api_models::EmissionSchedule> {
        self.update_emission_schedule_impl(id, request).await
    }

    async fn distribute_emission(
        &self,
        id: uuid::Uuid,
        request: platform_api_models::DistributeEmissionRequest,
    ) -> Result<()> {
        self.distribute_emission_impl(id, request).await
    }

    async fn calculate_emission(
        &self,
        request: platform_api_models::CalculateEmissionRequest,
    ) -> Result<platform_api_models::CalculateEmissionResponse> {
        self.calculate_emission_impl(request).await
    }

    async fn get_emission_aggregate(
        &self,
        period_start: chrono::DateTime<chrono::Utc>,
        period_end: chrono::DateTime<chrono::Utc>,
    ) -> Result<platform_api_models::EmissionAggregate> {
        self.get_emission_aggregate_impl(period_start, period_end)
            .await
    }

    async fn get_challenge_emission_metrics(
        &self,
        id: uuid::Uuid,
    ) -> Result<platform_api_models::ChallengeEmissionMetrics> {
        self.get_challenge_emission_metrics_impl(id).await
    }

    async fn get_validator_emission_metrics(
        &self,
        hotkey: &str,
    ) -> Result<platform_api_models::ValidatorEmissionMetrics> {
        self.get_validator_emission_metrics_impl(hotkey).await
    }

    async fn get_miner_emission_metrics(
        &self,
        hotkey: &str,
    ) -> Result<platform_api_models::MinerEmissionMetrics> {
        self.get_miner_emission_metrics_impl(hotkey).await
    }

    async fn get_emission_report(
        &self,
        period_start: chrono::DateTime<chrono::Utc>,
        period_end: chrono::DateTime<chrono::Utc>,
    ) -> Result<platform_api_models::EmissionReport> {
        self.get_emission_report_impl(period_start, period_end)
            .await
    }

    async fn list_pools(
        &self,
        validator_hotkey: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Result<platform_api_models::PoolListResponse> {
        self.list_pools_impl(validator_hotkey, page, per_page).await
    }

    async fn get_pool(&self, id: uuid::Uuid) -> Result<platform_api_models::Pool> {
        self.get_pool_impl(id).await
    }

    async fn create_pool(
        &self,
        validator_hotkey: &str,
        request: platform_api_models::CreatePoolRequest,
    ) -> Result<platform_api_models::Pool> {
        self.create_pool_impl(validator_hotkey, request).await
    }

    async fn update_pool(
        &self,
        id: uuid::Uuid,
        request: platform_api_models::UpdatePoolRequest,
    ) -> Result<platform_api_models::Pool> {
        self.update_pool_impl(id, request).await
    }

    async fn delete_pool(&self, id: uuid::Uuid) -> Result<()> {
        self.delete_pool_impl(id).await
    }

    async fn list_nodes(
        &self,
        pool_id: Option<uuid::Uuid>,
        page: u32,
        per_page: u32,
    ) -> Result<platform_api_models::NodeListResponse> {
        self.list_nodes_impl(pool_id, page, per_page).await
    }

    async fn get_node(&self, id: uuid::Uuid) -> Result<platform_api_models::Node> {
        self.get_node_impl(id).await
    }

    async fn add_node(
        &self,
        pool_id: uuid::Uuid,
        request: platform_api_models::AddNodeRequest,
    ) -> Result<platform_api_models::Node> {
        self.add_node_impl(pool_id, request).await
    }

    async fn update_node(
        &self,
        id: uuid::Uuid,
        request: platform_api_models::UpdateNodeRequest,
    ) -> Result<platform_api_models::Node> {
        self.update_node_impl(id, request).await
    }

    async fn delete_node(&self, id: uuid::Uuid) -> Result<()> {
        self.delete_node_impl(id).await
    }

    async fn get_pool_capacity(
        &self,
        pool_id: uuid::Uuid,
    ) -> Result<platform_api_models::PoolCapacitySummary> {
        self.get_pool_capacity_impl(pool_id).await
    }

    async fn get_vm_compose_config(
        &self,
        vm_type: &str,
    ) -> Result<platform_api_models::VmComposeConfig> {
        self.get_vm_compose_config_impl(vm_type).await
    }
}
