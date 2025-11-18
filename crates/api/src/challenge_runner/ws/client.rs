//! Challenge WebSocket client implementation

use super::types::ChallengeWsClient;
use anyhow::Result;

impl ChallengeWsClient {
    /// Create a new WebSocket client
    pub fn new(url: String, platform_api_id: String) -> Self {
        Self {
            url,
            platform_api_id,
            challenge_id: None,
            challenge_name: None,
            orm_gateway: None,
            migration_runner: None,
            schema_name: None,
            db_version_sender: None,
            migrations_sender: None,
            migrations_applied_receiver: None,
            migrations_applied_sender: None,
            validator_challenge_status: None,
            redis_client: None,
            compose_hash: None,
            validator_connections: None,
        }
    }

    /// Configure client with challenge information
    pub fn with_challenge(
        mut self,
        challenge_id: String,
        challenge_name: String,
        orm_gateway: std::sync::Arc<tokio::sync::RwLock<platform_api_orm_gateway::SecureORMGateway>>,
        migration_runner: Option<std::sync::Arc<crate::challenge_runner::migrations::MigrationRunner>>,
        schema_name: Option<std::sync::Arc<tokio::sync::RwLock<String>>>,
        db_version_sender: Option<
            std::sync::Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<Option<u32>>>>>,
        >,
        migrations_sender: Option<
            std::sync::Arc<
                tokio::sync::Mutex<
                    Option<
                        tokio::sync::oneshot::Sender<
                            Vec<crate::challenge_runner::migrations::Migration>,
                        >,
                    >,
                >,
            >,
        >,
        migrations_applied_receiver: Option<
            std::sync::Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
        >,
        migrations_applied_sender: Option<
            std::sync::Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        >,
        validator_challenge_status: Option<
            std::sync::Arc<
                tokio::sync::RwLock<
                    std::collections::HashMap<
                        String,
                        std::collections::HashMap<
                            String,
                            platform_api_models::ValidatorChallengeStatus,
                        >,
                    >,
                >,
            >,
        >,
        redis_client: Option<std::sync::Arc<crate::redis_client::RedisClient>>,
    ) -> Self {
        self.challenge_id = Some(challenge_id);
        self.challenge_name = Some(challenge_name);
        self.orm_gateway = Some(orm_gateway);
        self.migration_runner = migration_runner;
        self.schema_name = schema_name;
        self.db_version_sender = db_version_sender;
        self.migrations_sender = migrations_sender;
        self.migrations_applied_receiver = migrations_applied_receiver;
        self.migrations_applied_sender = migrations_applied_sender;
        self.validator_challenge_status = validator_challenge_status;
        self.redis_client = redis_client;
        self
    }

    /// Set compose_hash for this challenge
    pub fn with_compose_hash(mut self, compose_hash: String) -> Self {
        self.compose_hash = Some(compose_hash);
        self
    }
}

