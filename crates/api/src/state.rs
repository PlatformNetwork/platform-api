use std::sync::Arc;
use std::collections::HashMap;
use platform_api_storage::{StorageBackend, MemoryStorageBackend, ArtifactStorage};
use platform_api_attestation::AttestationService;
use platform_api_kbs::KeyBrokerService;
use platform_api_scheduler::SchedulerService;
use platform_api_builder::BuilderService;
use platform_api_models::{ChallengeSpec, ValidatorChallengeStatus};
use crate::security::PlatformSecurity;
use crate::orm_gateway::{SecureORMGateway, ORMGatewayConfig};
use crate::challenge_runner::ChallengeRunner;
use crate::models::JobCache;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use tracing::info;
use sqlx::PgPool;

/// Application state shared across all handlers
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn StorageBackend>,
    pub attestation: Arc<AttestationService>,
    pub kbs: Arc<KeyBrokerService>,
    pub scheduler: Arc<SchedulerService>,
    pub builder: Arc<BuilderService>,
    pub metrics: Arc<MetricsService>,
    pub config: Arc<AppConfig>,
    pub artifact_storage: Arc<ArtifactStorage>,
    pub security: Arc<PlatformSecurity>,
    pub validator_connections: Arc<tokio::sync::RwLock<HashMap<String, ValidatorConnection>>>,
    pub challenge_registry: Arc<tokio::sync::RwLock<HashMap<String, ChallengeSpec>>>, // Key: compose_hash
    pub validator_challenge_status: Arc<tokio::sync::RwLock<HashMap<String, HashMap<String, ValidatorChallengeStatus>>>>, // Key: validator_hotkey -> compose_hash
    pub database_pool: Option<Arc<PgPool>>, // PostgreSQL connection pool
    pub orm_gateway: Option<Arc<tokio::sync::RwLock<SecureORMGateway>>>, // ORM gateway for read-write queries (public routes from SDK)
    pub orm_gateway_readonly: Option<Arc<tokio::sync::RwLock<SecureORMGateway>>>, // ORM gateway for read-only queries (validator routes)
    pub challenge_runner: Option<Arc<ChallengeRunner>>, // Challenge runner for API mode
    pub job_cache: Arc<tokio::sync::RwLock<HashMap<String, JobCache>>>, // Key: job_id -> JobCache
}

/// Validator connection information
#[derive(Debug, Clone)]
pub struct ValidatorConnection {
    pub validator_hotkey: String,
    pub app_id: Option<String>,
    pub instance_id: Option<String>,
    pub compose_hash: String,
    pub connected_at: DateTime<Utc>,
    pub session_token: String,
    pub last_ping: DateTime<Utc>,
    pub message_sender: Option<Arc<tokio::sync::mpsc::Sender<String>>>, // Channel to send messages to validator WebSocket (via mpsc channel)
}

/// Application configuration
#[derive(Clone)]
pub struct AppConfig {
    pub server_port: u16,
    pub server_host: String,
    pub jwt_secret_ui: String, // Dedicated JWT secret for UI routes (separate from attestation JWT)
    pub database_url: String,
    pub storage_config: StorageConfig,
    pub attestation_config: AttestationConfig,
    pub kbs_config: KbsConfig,
    pub scheduler_config: SchedulerConfig,
    pub builder_config: BuilderConfig,
    pub metrics_config: MetricsConfig,
}

// Config types are now imported from their respective crates
use platform_api_storage::StorageConfig;
use platform_api_attestation::AttestationConfig;
use platform_api_kbs::KbsConfig;
use platform_api_scheduler::SchedulerConfig;
use platform_api_builder::BuilderConfig;

/// Metrics configuration
#[derive(Clone)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub port: u16,
    pub path: String,
    pub collect_interval: u64,
}

/// Metrics service
#[derive(Clone)]
pub struct MetricsService {
    pub metrics: String,
}

impl MetricsService {
    pub fn new(_config: &MetricsConfig) -> anyhow::Result<Self> {
        Ok(Self {
            metrics: "# platform-api metrics\n".to_string(),
        })
    }

    pub fn get_metrics(&self) -> anyhow::Result<String> {
        Ok(self.metrics.clone())
    }
}

impl AppState {
    pub async fn new(config: AppConfig) -> anyhow::Result<Self> {
        // Initialize storage backend based on configuration
        let (storage, database_pool) = if config.storage_config.backend_type == "postgres" {
            use platform_api_storage::PostgresStorageBackend;
            info!("Using PostgreSQL storage backend");
            let pg_backend = PostgresStorageBackend::new(&config.database_url).await?;
            let pool = pg_backend.get_pool().clone();
            (Arc::new(pg_backend) as Arc<dyn StorageBackend>, Some(Arc::new(pool)))
        } else {
            info!("Using memory storage backend");
            (Arc::new(MemoryStorageBackend::new(&config.storage_config)?) as Arc<dyn StorageBackend>, None)
        };
        
        let attestation = Arc::new(AttestationService::new(&config.attestation_config)?);
        let kbs = Arc::new(KeyBrokerService::new(&config.kbs_config)?);
        
        // Initialize scheduler with database pool if available
        let scheduler = if let Some(ref pool) = database_pool {
            Arc::new(SchedulerService::with_database(&config.scheduler_config, pool.clone())?)
        } else {
            Arc::new(SchedulerService::new(&config.scheduler_config)?)
        };
        
        let builder = Arc::new(BuilderService::new(&config.builder_config)?);
        let metrics = Arc::new(MetricsService::new(&config.metrics_config)?);
        let artifact_storage = Arc::new(ArtifactStorage::new());
        let security = Arc::new(PlatformSecurity::init_from_tdx().await?);
        let validator_connections = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let challenge_registry = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let validator_challenge_status = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let job_cache = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        
        // Initialize ORM gateways if database pool is available
        // Read-write gateway for direct SDK connections (public routes)
        let orm_gateway = database_pool.as_ref().map(|pool| {
            let orm_config = ORMGatewayConfig::read_write(); // Read-write for public routes
            Arc::new(tokio::sync::RwLock::new(SecureORMGateway::new(orm_config, (**pool).clone())))
        });
        
        // Read-only gateway for validator routes
        let orm_gateway_readonly = database_pool.as_ref().map(|pool| {
            let orm_config = ORMGatewayConfig::read_only(); // Read-only for validator
            Arc::new(tokio::sync::RwLock::new(SecureORMGateway::new(orm_config, (**pool).clone())))
        });

        Ok(Self {
            storage,
            attestation,
            kbs,
            scheduler,
            builder,
            metrics,
            config: Arc::new(config),
            artifact_storage,
            security,
            validator_connections,
            challenge_registry,
            validator_challenge_status,
            database_pool,
            orm_gateway,
            orm_gateway_readonly,
            challenge_runner: None, // Will be set by main.rs if enabled
            job_cache,
        })
    }
    
    /// Add a validator connection
    pub async fn add_validator_connection(&self, conn: ValidatorConnection) {
        let mut connections = self.validator_connections.write().await;
        connections.insert(conn.validator_hotkey.clone(), conn);
    }
    
    /// Get a validator connection
    pub async fn get_validator_connection(&self, hotkey: &str) -> Option<ValidatorConnection> {
        let connections = self.validator_connections.read().await;
        connections.get(hotkey).cloned()
    }
    
    /// Remove a validator connection
    pub async fn remove_validator_connection(&self, hotkey: &str) {
        let mut connections = self.validator_connections.write().await;
        connections.remove(hotkey);
    }
    
    /// List all connected validators
    pub async fn list_validator_connections(&self) -> Vec<ValidatorConnection> {
        let connections = self.validator_connections.read().await;
        connections.values().cloned().collect()
    }
    
    /// Add or update a challenge in the registry
    pub async fn register_challenge(&self, challenge: ChallengeSpec) {
        let mut registry = self.challenge_registry.write().await;
        registry.insert(challenge.compose_hash.clone(), challenge);
    }
    
    /// Get a challenge by compose_hash
    pub async fn get_challenge(&self, compose_hash: &str) -> Option<ChallengeSpec> {
        let registry = self.challenge_registry.read().await;
        registry.get(compose_hash).cloned()
    }
    
    /// List all registered challenges with auto-calculated weights
    pub async fn list_challenges(&self) -> Vec<ChallengeSpec> {
        let registry = self.challenge_registry.read().await;
        let registry_size = registry.len();
        tracing::info!("📋 list_challenges: registry contains {} challenges", registry_size);
        for (hash, challenge) in registry.iter() {
            tracing::info!("  - {} (hash: {})", challenge.name, hash);
        }
        let mut challenges: Vec<ChallengeSpec> = registry.values().cloned().collect();
        
        // Group challenges by mechanism_id
        use std::collections::HashMap;
        let mut mechanism_challenges: HashMap<u8, Vec<&mut ChallengeSpec>> = HashMap::new();
        
        for challenge in &mut challenges {
            mechanism_challenges
                .entry(challenge.mechanism_id)
                .or_insert_with(Vec::new)
                .push(challenge);
        }
        
        // Calculate weights for each mechanism
        for (_mechanism_id, mechanism_challs) in mechanism_challenges {
            let count = mechanism_challs.len();
            
            // Calculate automatic weights
            for challenge in mechanism_challs {
                if challenge.weight.is_none() {
                    if count == 1 {
                        // Single challenge on mechanism: weight = 1.0
                        challenge.weight = Some(1.0);
                    } else {
                        // Multiple challenges: divide 1.0 by count
                        challenge.weight = Some(1.0 / count as f64);
                    }
                }
            }
        }
        
        challenges
    }
    
    /// Remove a challenge from registry
    pub async fn remove_challenge(&self, compose_hash: &str) {
        let mut registry = self.challenge_registry.write().await;
        registry.remove(compose_hash);
    }
    
    /// Update validator challenge status
    pub async fn update_validator_challenge_status(&self, hotkey: &str, status: ValidatorChallengeStatus) {
        let mut status_map = self.validator_challenge_status.write().await;
        status_map.entry(hotkey.to_string())
            .or_insert_with(HashMap::new)
            .insert(status.compose_hash.clone(), status);
    }
    
    /// Get validator challenge status
    pub async fn get_validator_challenge_status(&self, hotkey: &str) -> Vec<ValidatorChallengeStatus> {
        let status_map = self.validator_challenge_status.read().await;
        status_map.get(hotkey)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }
    
    /// Get count of active validators for a specific compose_hash
    /// This counts validators that have the challenge in Active state
    pub async fn get_validator_count(&self, compose_hash: &str) -> usize {
        let status_map = self.validator_challenge_status.read().await;
        let mut count = 0;
        
        for (_hotkey, challenge_statuses) in status_map.iter() {
            if let Some(status) = challenge_statuses.get(compose_hash) {
                if matches!(status.state, platform_api_models::ValidatorChallengeState::Active) {
                    count += 1;
                }
            }
        }
        
        info!(
            compose_hash = compose_hash,
            validator_count = count,
            "Getting validator count for challenge"
        );
        
        count
    }
    
    /// Initialize security with TDX attestation
    pub async fn init_security_from_tdx(self) -> anyhow::Result<Self> {
        let security = Arc::new(PlatformSecurity::init_from_tdx().await?);
        
        Ok(Self {
            security,
            ..self
        })
    }
}


