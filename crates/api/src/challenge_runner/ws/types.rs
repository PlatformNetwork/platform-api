//! Types for WebSocket challenge client

use crate::redis_client::RedisClient;
use platform_api_orm_gateway::SecureORMGateway;
use std::sync::Arc;

/// Envelope used for encrypted WebSocket frames
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct EncryptedEnvelope {
    pub enc: String,
    pub nonce: String,      // base64(12 bytes)
    pub ciphertext: String, // base64
}

/// Connection state tracking for TDX verification enforcement
#[derive(Debug, Clone)]
pub enum ConnectionState {
    Unverified { nonce: [u8; 32], started: tokio::time::Instant },
    Verified { aead_key: [u8; 32] },
    Rejected { reason: String },
}

#[derive(Clone)]
pub struct ChallengeWsClient {
    pub url: String,
    pub platform_api_id: String, // Platform API identifier (instead of validator_hotkey)
    pub challenge_id: Option<String>, // Challenge ID for schema routing
    pub challenge_name: Option<String>, // Challenge name for schema naming
    pub orm_gateway: Option<Arc<tokio::sync::RwLock<SecureORMGateway>>>, // ORM Gateway for query execution
    pub migration_runner: Option<Arc<crate::challenge_runner::migrations::MigrationRunner>>, // Migration runner for applying migrations
    pub schema_name: Option<Arc<tokio::sync::RwLock<String>>>, // Schema name for migrations (will be computed from name + db_version, can be updated)
    pub db_version_sender:
        Option<Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<Option<u32>>>>>>, // Channel to send db_version back to caller
    pub migrations_sender: Option<
        Arc<
            tokio::sync::Mutex<
                Option<
                    tokio::sync::oneshot::Sender<
                        Vec<crate::challenge_runner::migrations::Migration>,
                    >,
                >,
            >,
        >,
    >, // Channel to send migrations back to caller
    pub migrations_applied_receiver:
        Option<Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>>, // Channel to receive signal that migrations are applied (from mod.rs)
    pub migrations_applied_sender:
        Option<Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>>, // Channel to send signal that migrations are applied (from WebSocket task)
    pub validator_challenge_status: Option<
        Arc<
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
    >, // For get_validator_count
    pub redis_client: Option<Arc<RedisClient>>, // Redis client for job progress logging
    pub compose_hash: Option<String>, // Compose hash for this challenge (used for validator filtering)
    pub validator_connections: Option<
        Arc<
            tokio::sync::RwLock<
                std::collections::HashMap<String, crate::state::ValidatorConnection>,
            >,
        >,
    >, // Validator connections for getting connected validators
}

impl ChallengeWsClient {
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

    pub fn with_challenge(
        mut self,
        challenge_id: String,
        challenge_name: String,
        orm_gateway: Arc<tokio::sync::RwLock<SecureORMGateway>>,
        migration_runner: Option<Arc<crate::challenge_runner::migrations::MigrationRunner>>,
        schema_name: Option<Arc<tokio::sync::RwLock<String>>>,
        db_version_sender: Option<
            Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<Option<u32>>>>>,
        >,
        migrations_sender: Option<
            Arc<
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
            Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
        >,
        migrations_applied_sender: Option<
            Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        >,
        validator_challenge_status: Option<
            Arc<
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
        redis_client: Option<Arc<RedisClient>>,
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
