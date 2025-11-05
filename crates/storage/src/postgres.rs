use anyhow::Result;
use sqlx::{PgPool, postgres::PgPoolOptions, FromRow};
use platform_api_models::*;
use uuid::Uuid;
use tracing::{info, error};
use super::StorageBackend;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

/// Database row for challenges table
#[derive(Debug, FromRow, Serialize, Deserialize)]
struct ChallengeRow {
    id: Uuid,
    name: String,
    compose_hash: String,
    compose_yaml: String,
    version: String,
    images: Vec<String>,
    resources: serde_json::Value,
    ports: serde_json::Value,
    env: serde_json::Value,
    emission_share: f64,
    mechanism_id: i16, // PostgreSQL SMALLINT maps to i16 in Rust, but we'll convert to u8
    weight: Option<f64>,
    description: Option<String>,
    mermaid_chart: Option<String>,
    github_repo: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// PostgreSQL storage backend
pub struct PostgresStorageBackend {
    pool: PgPool,
}

impl PostgresStorageBackend {
    pub fn get_pool(&self) -> &PgPool {
        &self.pool
    }
}

impl PostgresStorageBackend {
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
            )"
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
                || e.to_string().contains("prepared statement") {
                info!("⚠️  Skipping migrations (likely already applied or concurrent execution)");
            } else {
                return Err(anyhow::anyhow!("Failed to run migrations: {}", e));
            }
        } else {
            info!("✅ Database migrations completed");
        }
        
        Ok(Self { pool })
    }
}

#[async_trait::async_trait]
impl StorageBackend for PostgresStorageBackend {
    async fn list_challenges(&self, page: u32, per_page: u32, _status: Option<String>, _visibility: Option<String>) -> Result<ChallengeListResponse> {
        // For now, return empty list - challenges are managed in memory via challenge_registry
        Ok(ChallengeListResponse {
            challenges: vec![],
            total: 0,
            page,
            per_page,
        })
    }

    async fn get_challenge(&self, _id: Uuid) -> Result<ChallengeDetailResponse> {
        Err(anyhow::anyhow!("Challenge not found"))
    }

    async fn get_challenge_emissions(&self, id: Uuid) -> Result<EmissionSchedule> {
        let row = sqlx::query_as::<_, ChallengeRow>(
            "SELECT id, name, compose_hash, compose_yaml, version, images, resources, ports, env, emission_share, mechanism_id, weight, description, mermaid_chart, github_repo, created_at, updated_at FROM challenges WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Challenge not found"))?;

        Ok(EmissionSchedule {
            id: Id::from(row.id),
            emission_type: EmissionType::Challenge,
            challenge_id: Some(Id::from(row.id)),
            start_time: row.created_at,
            end_time: None,
            emission_rate: row.emission_share, // emission_share (0-1) est le poids du challenge
            total_amount: row.emission_share,
            distributed_amount: 0.0,
            status: EmissionStatus::Active,
            distribution_curve: DistributionCurve::Linear,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    async fn get_subnet_config(&self) -> Result<SubnetConfig> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn update_subnet_config(&self, _config: SubnetConfig) -> Result<SubnetConfig> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn validate_config(&self, _config: &UpdateConfigRequest) -> Result<ConfigValidationResult> {
        Ok(ConfigValidationResult {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
            suggestions: vec![],
        })
    }

    async fn create_config_backup(&self, _request: super::CreateBackupRequest) -> Result<ConfigBackup> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn restore_config(&self, _request: RestoreConfigRequest) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn list_config_backups(&self) -> Result<Vec<ConfigBackup>> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_config_backup(&self, _id: Uuid) -> Result<ConfigBackup> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_config_history(&self) -> Result<Vec<ConfigChangeLog>> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn list_emission_schedules(&self, status: Option<String>, emission_type: Option<String>, challenge_id: Option<Uuid>) -> Result<Vec<EmissionSchedule>> {
        let mut query = "SELECT id, name, compose_hash, compose_yaml, version, images, resources, ports, env, emission_share, mechanism_id, weight, description, mermaid_chart, github_repo, created_at, updated_at FROM challenges WHERE 1=1".to_string();
        let mut bind_counter = 1;

        if let Some(cid) = challenge_id {
            query.push_str(&format!(" AND id = ${}", bind_counter));
            bind_counter += 1;
        }

        let mut query_builder = sqlx::query_as::<_, ChallengeRow>(&query);
        if let Some(cid) = challenge_id {
            query_builder = query_builder.bind(cid);
        }

        let rows = query_builder.fetch_all(&self.pool).await?;

        let schedules: Vec<EmissionSchedule> = rows.into_iter().map(|row| {
            EmissionSchedule {
                id: Id::from(row.id),
                emission_type: EmissionType::Challenge,
                challenge_id: Some(Id::from(row.id)),
                start_time: row.created_at,
                end_time: None,
                emission_rate: row.emission_share,
                total_amount: row.emission_share,
                distributed_amount: 0.0,
                status: EmissionStatus::Active,
                distribution_curve: DistributionCurve::Linear,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }
        }).collect();

        // Filter by status if provided
        let schedules = if let Some(status_str) = status {
            let status_enum = match status_str.as_str() {
                "scheduled" => EmissionStatus::Scheduled,
                "active" => EmissionStatus::Active,
                "completed" => EmissionStatus::Completed,
                "paused" => EmissionStatus::Paused,
                "cancelled" => EmissionStatus::Cancelled,
                _ => return Ok(schedules),
            };
            schedules.into_iter().filter(|s| s.status == status_enum).collect()
        } else {
            schedules
        };

        Ok(schedules)
    }

    async fn get_emission_schedule(&self, id: Uuid) -> Result<EmissionSchedule> {
        self.get_challenge_emissions(id).await
    }

    async fn create_emission_schedule(&self, request: CreateEmissionScheduleRequest) -> Result<EmissionSchedule> {
        // For emissions, update the challenge's emission_share
        if let Some(challenge_id) = request.challenge_id {
            // Validate emission_rate is between 0 and 1
            if request.emission_rate < 0.0 || request.emission_rate > 1.0 {
                return Err(anyhow::anyhow!("Emission rate must be between 0.0 and 1.0"));
            }

            let now = Utc::now();
            sqlx::query(
                "UPDATE challenges SET emission_share = $1, updated_at = $2 WHERE id = $3"
            )
            .bind(request.emission_rate)
            .bind(now)
            .bind(challenge_id)
            .execute(&self.pool)
            .await?;

            self.get_challenge_emissions(challenge_id).await
        } else {
            Err(anyhow::anyhow!("challenge_id is required for emission schedules"))
        }
    }

    async fn update_emission_schedule(&self, id: Uuid, request: UpdateEmissionScheduleRequest) -> Result<EmissionSchedule> {
        if let Some(emission_rate) = request.emission_rate {
            if emission_rate < 0.0 || emission_rate > 1.0 {
                return Err(anyhow::anyhow!("Emission rate must be between 0.0 and 1.0"));
            }

            sqlx::query(
                "UPDATE challenges SET emission_share = $1, updated_at = $2 WHERE id = $3"
            )
            .bind(emission_rate)
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        }

        // Status is managed but not stored directly in challenges table
        // Challenges are always considered "Active" in this model

        self.get_challenge_emissions(id).await
    }

    async fn distribute_emission(&self, _id: Uuid, _request: DistributeEmissionRequest) -> Result<()> {
        // Distribution is handled by the weights mechanism, so we just log
        info!("Emission distribution requested for schedule {}, but distribution is handled automatically through weight aggregation", _id);
        Ok(())
    }

    async fn calculate_emission(&self, request: CalculateEmissionRequest) -> Result<CalculateEmissionResponse> {
        let mut total_emission = 0.0;
        let mut breakdown = BTreeMap::new();

        if let Some(challenge_id) = request.challenge_id {
            let schedule = self.get_challenge_emissions(challenge_id).await?;
            total_emission = schedule.emission_rate;
            breakdown.insert(format!("challenge_{}", challenge_id), schedule.emission_rate);
        } else {
            // Calculate total emissions for all challenges if no specific challenge
            let schedules = self.list_emission_schedules(None, None, None).await?;
            for schedule in schedules {
                total_emission += schedule.emission_rate;
                if let Some(cid) = schedule.challenge_id {
                    breakdown.insert(format!("challenge_{}", cid), schedule.emission_rate);
                }
            }
        }

        Ok(CalculateEmissionResponse {
            total_emission,
            breakdown,
            distributions: vec![],
            metrics: EmissionMetrics {
                participation_rate: 1.0,
                quality_score: 1.0,
                efficiency_score: 1.0,
                fairness_score: 1.0,
                sustainability_score: 1.0,
            },
        })
    }

    async fn get_emission_aggregate(&self, period_start: chrono::DateTime<chrono::Utc>, period_end: chrono::DateTime<chrono::Utc>) -> Result<EmissionAggregate> {
        let schedules = self.list_emission_schedules(None, None, None).await?;
        
        let mut total_emissions = 0.0;
        let mut challenge_emissions = 0.0;

        for schedule in &schedules {
            total_emissions += schedule.emission_rate;
            if schedule.emission_type == EmissionType::Challenge {
                challenge_emissions += schedule.emission_rate;
            }
        }

        Ok(EmissionAggregate {
            total_emissions,
            challenge_emissions,
            validator_emissions: 0.0,
            miner_emissions: 0.0,
            owner_emissions: 0.0,
            network_emissions: 0.0,
            period_start,
            period_end,
            distributions: vec![],
        })
    }

    async fn get_challenge_emission_metrics(&self, id: Uuid) -> Result<ChallengeEmissionMetrics> {
        let schedule = self.get_challenge_emissions(id).await?;

        Ok(ChallengeEmissionMetrics {
            challenge_id: Id::from(id),
            total_emission: schedule.total_amount,
            distributed_emission: schedule.distributed_amount,
            pending_emission: schedule.total_amount - schedule.distributed_amount,
            emission_rate: schedule.emission_rate,
            participation_score: 1.0,
            quality_score: 1.0,
            efficiency_score: 1.0,
            last_distribution: None,
            next_distribution: None,
        })
    }

    async fn get_validator_emission_metrics(&self, _hotkey: &str) -> Result<ValidatorEmissionMetrics> {
        // Validators don't have direct emissions in this model
        Ok(ValidatorEmissionMetrics {
            validator_hotkey: _hotkey.to_string(),
            total_emission: 0.0,
            distributed_emission: 0.0,
            pending_emission: 0.0,
            performance_score: 1.0,
            uptime_score: 1.0,
            accuracy_score: 1.0,
            efficiency_score: 1.0,
            last_distribution: None,
            next_distribution: None,
        })
    }

    async fn get_miner_emission_metrics(&self, _hotkey: &str) -> Result<MinerEmissionMetrics> {
        // Miners don't have direct emissions in this model
        Ok(MinerEmissionMetrics {
            miner_hotkey: _hotkey.to_string(),
            total_emission: 0.0,
            distributed_emission: 0.0,
            pending_emission: 0.0,
            submission_score: 1.0,
            quality_score: 1.0,
            participation_score: 1.0,
            innovation_score: 1.0,
            last_distribution: None,
            next_distribution: None,
        })
    }

    async fn get_emission_report(&self, period_start: chrono::DateTime<chrono::Utc>, period_end: chrono::DateTime<chrono::Utc>) -> Result<EmissionReport> {
        let aggregate = self.get_emission_aggregate(period_start, period_end).await?;
        let schedules = self.list_emission_schedules(None, None, None).await?;

        Ok(EmissionReport {
            period_start,
            period_end,
            total_emissions: aggregate.total_emissions,
            schedule_count: schedules.len() as u32,
            distribution_count: 0,
            recipient_count: 0,
            avg_distribution_amount: 0.0,
            top_recipients: vec![],
            emission_trends: BTreeMap::new(),
        })
    }

    async fn list_pools(&self, _validator_hotkey: Option<&str>, _page: u32, _per_page: u32) -> Result<PoolListResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_pool(&self, _id: Uuid) -> Result<Pool> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn create_pool(&self, _validator_hotkey: &str, _request: CreatePoolRequest) -> Result<Pool> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn update_pool(&self, _id: Uuid, _request: UpdatePoolRequest) -> Result<Pool> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn delete_pool(&self, _id: Uuid) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn list_nodes(&self, _pool_id: Option<Uuid>, _page: u32, _per_page: u32) -> Result<NodeListResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_node(&self, _id: Uuid) -> Result<Node> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn add_node(&self, _pool_id: Uuid, _request: AddNodeRequest) -> Result<Node> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn update_node(&self, _id: Uuid, _request: UpdateNodeRequest) -> Result<Node> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn delete_node(&self, _id: Uuid) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_pool_capacity(&self, _pool_id: Uuid) -> Result<PoolCapacitySummary> {
        Err(anyhow::anyhow!("Not implemented"))
    }
}

