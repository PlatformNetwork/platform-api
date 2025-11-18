//! Challenge runner struct and constructor

use super::types::{ChallengeInstance, ChallengeRunnerConfig};
use crate::challenge_runner::{CvmManager, MigrationRunner};
use platform_api_orm_gateway::SecureORMGateway;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

/// Challenge runner for API mode execution
pub struct ChallengeRunner {
    pub config: ChallengeRunnerConfig,
    pub challenges: tokio::sync::RwLock<HashMap<String, ChallengeInstance>>, // Key: compose_hash
    pub db_pool: PgPool,
    pub migration_runner: MigrationRunner,
    pub cvm_manager: CvmManager,
    pub orm_gateway: Option<Arc<tokio::sync::RwLock<SecureORMGateway>>>, // ORM gateway for query bridge
    pub validator_challenge_status: Option<
        Arc<
            tokio::sync::RwLock<
                HashMap<
                    String,
                    HashMap<String, platform_api_models::ValidatorChallengeStatus>,
                >,
            >,
        >,
    >, // Validator challenge status for get_validator_count
    pub redis_client: Option<Arc<crate::redis_client::RedisClient>>, // Redis client for job progress logging
    pub validator_connections: Option<
        Arc<
            tokio::sync::RwLock<
                HashMap<String, crate::state::ValidatorConnection>,
            >,
        >,
    >, // Validator connections for getting connected validators
}

impl ChallengeRunner {
    pub fn new(
        config: ChallengeRunnerConfig,
        db_pool: PgPool,
        orm_gateway: Option<Arc<tokio::sync::RwLock<SecureORMGateway>>>,
        validator_challenge_status: Option<
            Arc<
                tokio::sync::RwLock<
                    HashMap<
                        String,
                        HashMap<String, platform_api_models::ValidatorChallengeStatus>,
                    >,
                >,
            >,
        >,
        redis_client: Option<Arc<crate::redis_client::RedisClient>>,
        validator_connections: Option<
            Arc<
                tokio::sync::RwLock<
                    HashMap<String, crate::state::ValidatorConnection>,
                >,
            >,
        >,
    ) -> Self {
        let migration_runner = MigrationRunner::new(db_pool.clone());
        let cvm_manager = CvmManager::new(config.vmm_url.clone(), config.gateway_url.clone());

        Self {
            config,
            challenges: tokio::sync::RwLock::new(HashMap::new()),
            db_pool,
            migration_runner,
            cvm_manager,
            orm_gateway,
            validator_challenge_status,
            redis_client,
            validator_connections,
        }
    }
}

