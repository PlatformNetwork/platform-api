use anyhow::Result;
use platform_api_models::*;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
pub struct CreateBackupRequest {
    pub reason: String,
    pub tags: Option<Vec<String>>,
}

mod config;
pub use config::*;

mod encryption;
pub use encryption::*;

mod artifacts;
pub use artifacts::*;

mod postgres;
pub use postgres::*;

/// Storage backend trait
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync {
    async fn list_challenges(
        &self,
        page: u32,
        per_page: u32,
        status: Option<String>,
        visibility: Option<String>,
    ) -> Result<ChallengeListResponse>;
    async fn get_challenge(&self, id: Uuid) -> Result<ChallengeDetailResponse>;
    async fn get_challenge_emissions(&self, id: Uuid) -> Result<EmissionSchedule>;
    async fn get_subnet_config(&self) -> Result<SubnetConfig>;
    async fn update_subnet_config(&self, _config: SubnetConfig) -> Result<SubnetConfig>;
    async fn validate_config(
        &self,
        _config: &UpdateConfigRequest,
    ) -> Result<ConfigValidationResult>;
    async fn create_config_backup(&self, _request: CreateBackupRequest) -> Result<ConfigBackup>;
    async fn restore_config(&self, _request: RestoreConfigRequest) -> Result<()>;
    async fn list_config_backups(&self) -> Result<Vec<ConfigBackup>>;
    async fn get_config_backup(&self, _id: Uuid) -> Result<ConfigBackup>;
    async fn get_config_history(&self) -> Result<Vec<ConfigChangeLog>>;
    async fn list_emission_schedules(
        &self,
        _status: Option<String>,
        _emission_type: Option<String>,
        _challenge_id: Option<Uuid>,
    ) -> Result<Vec<EmissionSchedule>>;
    async fn get_emission_schedule(&self, _id: Uuid) -> Result<EmissionSchedule>;
    async fn create_emission_schedule(
        &self,
        _request: CreateEmissionScheduleRequest,
    ) -> Result<EmissionSchedule>;
    async fn update_emission_schedule(
        &self,
        _id: Uuid,
        _request: UpdateEmissionScheduleRequest,
    ) -> Result<EmissionSchedule>;
    async fn distribute_emission(
        &self,
        _id: Uuid,
        _request: DistributeEmissionRequest,
    ) -> Result<()>;
    async fn calculate_emission(
        &self,
        _request: CalculateEmissionRequest,
    ) -> Result<CalculateEmissionResponse>;
    async fn get_emission_aggregate(
        &self,
        _period_start: chrono::DateTime<chrono::Utc>,
        _period_end: chrono::DateTime<chrono::Utc>,
    ) -> Result<EmissionAggregate>;
    async fn get_challenge_emission_metrics(&self, _id: Uuid) -> Result<ChallengeEmissionMetrics>;
    async fn get_validator_emission_metrics(
        &self,
        _hotkey: &str,
    ) -> Result<ValidatorEmissionMetrics>;
    async fn get_miner_emission_metrics(&self, _hotkey: &str) -> Result<MinerEmissionMetrics>;
    async fn get_emission_report(
        &self,
        _period_start: chrono::DateTime<chrono::Utc>,
        _period_end: chrono::DateTime<chrono::Utc>,
    ) -> Result<EmissionReport>;

    // Pool methods
    async fn list_pools(
        &self,
        validator_hotkey: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Result<PoolListResponse>;
    async fn get_pool(&self, id: Uuid) -> Result<Pool>;
    async fn create_pool(&self, validator_hotkey: &str, request: CreatePoolRequest)
        -> Result<Pool>;
    async fn update_pool(&self, id: Uuid, request: UpdatePoolRequest) -> Result<Pool>;
    async fn delete_pool(&self, id: Uuid) -> Result<()>;

    // Node methods
    async fn list_nodes(
        &self,
        pool_id: Option<Uuid>,
        page: u32,
        per_page: u32,
    ) -> Result<NodeListResponse>;
    async fn get_node(&self, id: Uuid) -> Result<Node>;
    async fn add_node(&self, pool_id: Uuid, request: AddNodeRequest) -> Result<Node>;
    async fn update_node(&self, id: Uuid, request: UpdateNodeRequest) -> Result<Node>;
    async fn delete_node(&self, id: Uuid) -> Result<()>;

    // Capacity methods
    async fn get_pool_capacity(&self, pool_id: Uuid) -> Result<PoolCapacitySummary>;
}

/// Basic storage backend implementation
pub struct MemoryStorageBackend {
    config: StorageConfig,
    subnet_config: tokio::sync::RwLock<Option<SubnetConfig>>,
    pools: tokio::sync::RwLock<std::collections::HashMap<Uuid, Pool>>,
    nodes: tokio::sync::RwLock<std::collections::HashMap<Uuid, Node>>,
}

impl MemoryStorageBackend {
    pub fn new(config: &StorageConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            subnet_config: tokio::sync::RwLock::new(None),
            pools: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            nodes: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        })
    }
}

#[async_trait::async_trait]
impl StorageBackend for MemoryStorageBackend {
    async fn list_challenges(
        &self,
        page: u32,
        per_page: u32,
        _status: Option<String>,
        _visibility: Option<String>,
    ) -> Result<ChallengeListResponse> {
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

    async fn get_challenge_emissions(&self, _id: Uuid) -> Result<EmissionSchedule> {
        Err(anyhow::anyhow!("Emissions not found"))
    }

    async fn get_subnet_config(&self) -> Result<SubnetConfig> {
        let config = self.subnet_config.read().await;
        config
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Config not found"))
    }

    async fn update_subnet_config(&self, config: SubnetConfig) -> Result<SubnetConfig> {
        let mut stored = self.subnet_config.write().await;
        *stored = Some(config.clone());
        Ok(config)
    }

    async fn validate_config(
        &self,
        _config: &UpdateConfigRequest,
    ) -> Result<ConfigValidationResult> {
        Ok(ConfigValidationResult {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
            suggestions: vec![],
        })
    }

    async fn create_config_backup(&self, _request: CreateBackupRequest) -> Result<ConfigBackup> {
        use sha2::{Digest, Sha256};

        let config = self.get_subnet_config().await?;
        let config_json = serde_json::to_string(&config)?;
        let mut hasher = Sha256::new();
        hasher.update(config_json.as_bytes());
        let checksum = hex::encode(hasher.finalize());

        Ok(ConfigBackup {
            id: Uuid::new_v4(),
            config,
            created_at: chrono::Utc::now(),
            created_by: "system".to_string(),
            version: 1,
            checksum,
        })
    }

    async fn restore_config(&self, _request: RestoreConfigRequest) -> Result<()> {
        Ok(())
    }

    async fn list_config_backups(&self) -> Result<Vec<ConfigBackup>> {
        Ok(vec![])
    }

    async fn get_config_backup(&self, _id: Uuid) -> Result<ConfigBackup> {
        Err(anyhow::anyhow!("Not found"))
    }

    async fn get_config_history(&self) -> Result<Vec<ConfigChangeLog>> {
        Ok(vec![])
    }

    async fn list_emission_schedules(
        &self,
        _status: Option<String>,
        _emission_type: Option<String>,
        _challenge_id: Option<Uuid>,
    ) -> Result<Vec<EmissionSchedule>> {
        Ok(vec![])
    }

    async fn get_emission_schedule(&self, _id: Uuid) -> Result<EmissionSchedule> {
        Ok(EmissionSchedule::default())
    }

    async fn create_emission_schedule(
        &self,
        _request: CreateEmissionScheduleRequest,
    ) -> Result<EmissionSchedule> {
        Ok(EmissionSchedule::default())
    }

    async fn update_emission_schedule(
        &self,
        _id: Uuid,
        _request: UpdateEmissionScheduleRequest,
    ) -> Result<EmissionSchedule> {
        Ok(EmissionSchedule::default())
    }

    async fn distribute_emission(
        &self,
        _id: Uuid,
        _request: DistributeEmissionRequest,
    ) -> Result<()> {
        Ok(())
    }

    async fn calculate_emission(
        &self,
        _request: CalculateEmissionRequest,
    ) -> Result<CalculateEmissionResponse> {
        Ok(CalculateEmissionResponse {
            total_emission: 0.0,
            breakdown: std::collections::BTreeMap::new(),
            distributions: vec![],
            metrics: EmissionMetrics {
                participation_rate: 0.0,
                quality_score: 0.0,
                efficiency_score: 0.0,
                fairness_score: 0.0,
                sustainability_score: 0.0,
            },
        })
    }

    async fn get_emission_aggregate(
        &self,
        period_start: chrono::DateTime<chrono::Utc>,
        period_end: chrono::DateTime<chrono::Utc>,
    ) -> Result<EmissionAggregate> {
        Ok(EmissionAggregate {
            period_start,
            period_end,
            total_emissions: 0.0,
            challenge_emissions: 0.0,
            validator_emissions: 0.0,
            miner_emissions: 0.0,
            owner_emissions: 0.0,
            network_emissions: 0.0,
            distributions: vec![],
        })
    }

    async fn get_challenge_emission_metrics(&self, id: Uuid) -> Result<ChallengeEmissionMetrics> {
        Ok(ChallengeEmissionMetrics {
            challenge_id: id,
            total_emission: 0.0,
            distributed_emission: 0.0,
            pending_emission: 0.0,
            emission_rate: 0.0,
            participation_score: 0.0,
            quality_score: 0.0,
            efficiency_score: 0.0,
            last_distribution: None,
            next_distribution: None,
        })
    }

    async fn get_validator_emission_metrics(
        &self,
        hotkey: &str,
    ) -> Result<ValidatorEmissionMetrics> {
        Ok(ValidatorEmissionMetrics {
            validator_hotkey: hotkey.to_string(),
            total_emission: 0.0,
            distributed_emission: 0.0,
            pending_emission: 0.0,
            performance_score: 0.0,
            uptime_score: 0.0,
            accuracy_score: 0.0,
            efficiency_score: 0.0,
            last_distribution: None,
            next_distribution: None,
        })
    }

    async fn get_miner_emission_metrics(&self, hotkey: &str) -> Result<MinerEmissionMetrics> {
        Ok(MinerEmissionMetrics {
            miner_hotkey: hotkey.to_string(),
            total_emission: 0.0,
            distributed_emission: 0.0,
            pending_emission: 0.0,
            submission_score: 0.0,
            quality_score: 0.0,
            participation_score: 0.0,
            innovation_score: 0.0,
            last_distribution: None,
            next_distribution: None,
        })
    }

    async fn get_emission_report(
        &self,
        period_start: chrono::DateTime<chrono::Utc>,
        period_end: chrono::DateTime<chrono::Utc>,
    ) -> Result<EmissionReport> {
        Ok(EmissionReport {
            period_start,
            period_end,
            total_emissions: 0.0,
            schedule_count: 0,
            distribution_count: 0,
            recipient_count: 0,
            avg_distribution_amount: 0.0,
            top_recipients: vec![],
            emission_trends: std::collections::BTreeMap::new(),
        })
    }

    // Pool implementations
    async fn list_pools(
        &self,
        validator_hotkey: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Result<PoolListResponse> {
        let pools = self.pools.read().await;
        let mut filtered: Vec<Pool> = pools.values().cloned().collect();

        if let Some(hotkey) = validator_hotkey {
            filtered.retain(|p| p.validator_hotkey == hotkey);
        }

        let total = filtered.len() as u64;
        let start = ((page - 1) * per_page) as usize;
        let end = (start + per_page as usize).min(filtered.len());

        Ok(PoolListResponse {
            pools: filtered.into_iter().skip(start).take(end - start).collect(),
            total,
            page,
            per_page,
        })
    }

    async fn get_pool(&self, id: Uuid) -> Result<Pool> {
        let pools = self.pools.read().await;
        pools
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Pool not found"))
    }

    async fn create_pool(
        &self,
        validator_hotkey: &str,
        request: CreatePoolRequest,
    ) -> Result<Pool> {
        let mut pools = self.pools.write().await;
        let pool = Pool {
            id: Uuid::new_v4(),
            validator_hotkey: validator_hotkey.to_string(),
            name: request.name,
            description: request.description,
            autoscale_policy: request.autoscale_policy.unwrap_or_default(),
            region: request.region,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        pools.insert(pool.id, pool.clone());
        Ok(pool)
    }

    async fn update_pool(&self, id: Uuid, request: UpdatePoolRequest) -> Result<Pool> {
        let mut pools = self.pools.write().await;
        let pool = pools
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Pool not found"))?;

        if let Some(name) = request.name {
            pool.name = name;
        }
        if let Some(description) = request.description {
            pool.description = Some(description);
        }
        if let Some(autoscale_policy) = request.autoscale_policy {
            pool.autoscale_policy = autoscale_policy;
        }
        if let Some(region) = request.region {
            pool.region = Some(region);
        }
        pool.updated_at = chrono::Utc::now();

        Ok(pool.clone())
    }

    async fn delete_pool(&self, id: Uuid) -> Result<()> {
        let mut pools = self.pools.write().await;
        pools
            .remove(&id)
            .ok_or_else(|| anyhow::anyhow!("Pool not found"))?;
        Ok(())
    }

    // Node implementations
    async fn list_nodes(
        &self,
        pool_id: Option<Uuid>,
        page: u32,
        per_page: u32,
    ) -> Result<NodeListResponse> {
        let nodes = self.nodes.read().await;
        let mut filtered: Vec<Node> = nodes.values().cloned().collect();

        if let Some(pid) = pool_id {
            filtered.retain(|n| n.pool_id == pid);
        }

        let total = filtered.len() as u64;
        let start = ((page - 1) * per_page) as usize;
        let end = (start + per_page as usize).min(filtered.len());

        Ok(NodeListResponse {
            nodes: filtered.into_iter().skip(start).take(end - start).collect(),
            total,
            page,
            per_page,
        })
    }

    async fn get_node(&self, id: Uuid) -> Result<Node> {
        let nodes = self.nodes.read().await;
        nodes
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Node not found"))
    }

    async fn add_node(&self, pool_id: Uuid, request: AddNodeRequest) -> Result<Node> {
        let mut nodes = self.nodes.write().await;
        let node = Node {
            id: Uuid::new_v4(),
            pool_id,
            name: request.name,
            vmm_url: request.vmm_url,
            capacity: request.capacity,
            health: NodeHealth::default(),
            metadata: request.metadata.unwrap_or_default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        nodes.insert(node.id, node.clone());
        Ok(node)
    }

    async fn update_node(&self, id: Uuid, request: UpdateNodeRequest) -> Result<Node> {
        let mut nodes = self.nodes.write().await;
        let node = nodes
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Node not found"))?;

        if let Some(name) = request.name {
            node.name = name;
        }
        if let Some(vmm_url) = request.vmm_url {
            node.vmm_url = vmm_url;
        }
        if let Some(capacity) = request.capacity {
            node.capacity = capacity;
        }
        if let Some(metadata) = request.metadata {
            node.metadata = metadata;
        }
        node.updated_at = chrono::Utc::now();

        Ok(node.clone())
    }

    async fn delete_node(&self, id: Uuid) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        nodes
            .remove(&id)
            .ok_or_else(|| anyhow::anyhow!("Node not found"))?;
        Ok(())
    }

    // Capacity implementation
    async fn get_pool_capacity(&self, pool_id: Uuid) -> Result<PoolCapacitySummary> {
        let nodes = self.nodes.read().await;
        let pool_nodes: Vec<&Node> = nodes.values().filter(|n| n.pool_id == pool_id).collect();

        let healthy_nodes = pool_nodes
            .iter()
            .filter(|n| matches!(n.health.status, HealthStatus::Healthy))
            .count() as u32;

        let total_cpu: u32 = pool_nodes.iter().map(|n| n.capacity.total_cpu).sum();
        let available_cpu: u32 = pool_nodes.iter().map(|n| n.capacity.available_cpu).sum();
        let total_memory_gb: u32 = pool_nodes.iter().map(|n| n.capacity.total_memory_gb).sum();
        let available_memory_gb: u32 = pool_nodes
            .iter()
            .map(|n| n.capacity.available_memory_gb)
            .sum();
        let has_tdx = pool_nodes.iter().any(|n| n.capacity.has_tdx);
        let gpu_count: u32 = pool_nodes.iter().map(|n| n.capacity.gpu_count).sum();

        Ok(PoolCapacitySummary {
            pool_id,
            total_nodes: pool_nodes.len() as u32,
            healthy_nodes,
            total_cpu,
            available_cpu,
            total_memory_gb,
            available_memory_gb,
            has_tdx,
            gpu_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_storage_backend() {
        let config = StorageConfig::default();
        let backend = MemoryStorageBackend::new(&config).unwrap();
        let result = backend.list_challenges(1, 20, None, None).await.unwrap();
        assert_eq!(result.total, 0);
    }
}
