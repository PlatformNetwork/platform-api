//! Core challenge runner functionality and orchestration

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

use super::runner::types::{ChallengeRunnerConfig, ChallengeInstance, ChallengeMetadata, ChallengeStatus};
use super::cvm_manager::CvmManager;
use super::migrations::MigrationRunner;
use super::challenge_ws::ChallengeWsClient;

/// Core challenge runner with modular components
pub struct ChallengeRunnerCore {
    pub config: ChallengeRunnerConfig,
    pub challenges: tokio::sync::RwLock<HashMap<String, ChallengeInstance>>, // Key: compose_hash
    pub db_pool: PgPool,
    pub migration_runner: MigrationRunner,
    pub cvm_manager: CvmManager,
    pub orm_gateway: Option<Arc<tokio::sync::RwLock<platform_api_orm_gateway::SecureORMGateway>>>,
    pub validator_challenge_status: Option<
        Arc<
            tokio::sync::RwLock<
                HashMap<
                    String,
                    HashMap<String, platform_api_models::ValidatorChallengeStatus>,
                >,
            >,
        >,
    >,
    pub redis_client: Option<Arc<crate::redis_client::RedisClient>>,
    pub validator_connections: Option<
        Arc<
            tokio::sync::RwLock<
                HashMap<String, crate::state::ValidatorConnection>,
            >,
        >,
    >,
}

impl ChallengeRunnerCore {
    /// Create new challenge runner core
    pub fn new(
        config: ChallengeRunnerConfig,
        db_pool: PgPool,
        orm_gateway: Option<Arc<tokio::sync::RwLock<platform_api_orm_gateway::SecureORMGateway>>>,
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

    /// Run a challenge in API mode by challenge_id (legacy method)
    pub async fn run_challenge(&self, challenge_id: &str) -> Result<()> {
        // Get challenge metadata to find compose_hash
        let challenge = self.get_challenge_metadata(challenge_id).await?;
        self.run_challenge_by_compose_hash(&challenge.compose_hash)
            .await
    }

    /// Run a challenge in API mode by compose_hash
    /// If challenge_spec is provided, uses that data instead of querying the database
    /// If env_vars is provided, injects those environment variables into the challenge
    pub async fn run_challenge_by_compose_hash_with_spec(
        &self,
        compose_hash: &str,
        challenge_spec: Option<Value>,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<()> {
        info!("Starting challenge execution for compose_hash: {}", compose_hash);

        // Validate challenge is not already running
        {
            let challenges = self.challenges.read().await;
            if challenges.contains_key(compose_hash) {
                warn!("Challenge {} is already running", compose_hash);
                return Err(anyhow::anyhow!("Challenge is already running"));
            }
        }

        // Get or use provided challenge metadata
        let challenge_metadata = if let Some(spec) = challenge_spec {
            ChallengeMetadata::from_spec(spec, compose_hash.to_string())?
        } else {
            self.get_challenge_by_compose_hash(compose_hash).await?
        };

        // Create challenge instance
        let instance = ChallengeInstance::new(
            challenge_metadata,
            self.config.clone(),
            env_vars.unwrap_or_default(),
        );

        // Store challenge instance
        {
            let mut challenges = self.challenges.write().await;
            challenges.insert(compose_hash.to_string(), instance.clone());
        }

        // Execute challenge
        let result = self.execute_challenge(instance).await;

        // Clean up challenge instance
        {
            let mut challenges = self.challenges.write().await;
            challenges.remove(compose_hash);
        }

        match result {
            Ok(_) => {
                info!("Challenge {} completed successfully", compose_hash);
                Ok(())
            }
            Err(e) => {
                error!("Challenge {} failed: {}", compose_hash, e);
                Err(e)
            }
        }
    }

    /// Execute a challenge instance
    async fn execute_challenge(&self, instance: ChallengeInstance) -> Result<()> {
        let compose_hash = instance.metadata.compose_hash.clone();

        // Step 1: Setup CVM environment
        info!("Setting up CVM environment for challenge: {}", compose_hash);
        let cvm_info = self.cvm_manager.setup_cvm(&instance).await
            .context("Failed to setup CVM environment")?;

        // Step 2: Run migrations if needed
        info!("Running migrations for challenge: {}", compose_hash);
        self.migration_runner.run_migrations(&instance, &cvm_info).await
            .context("Failed to run migrations")?;

        // Step 3: Establish WebSocket connection to challenge
        info!("Establishing WebSocket connection to challenge: {}", compose_hash);
        let ws_client = self.create_websocket_client(&instance, &cvm_info).await?;
        
        // Step 4: Run challenge via WebSocket
        info!("Running challenge via WebSocket: {}", compose_hash);
        self.run_challenge_via_websocket(ws_client, &instance).await
            .context("Failed to run challenge via WebSocket")?;

        // Step 5: Cleanup CVM environment
        info!("Cleaning up CVM environment for challenge: {}", compose_hash);
        self.cvm_manager.cleanup_cvm(&cvm_info).await
            .context("Failed to cleanup CVM environment")?;

        Ok(())
    }

    /// Create WebSocket client for challenge communication
    async fn create_websocket_client(
        &self,
        instance: &ChallengeInstance,
        cvm_info: &super::cvm_manager::CvmInfo,
    ) -> Result<ChallengeWsClient> {
        let ws_url = format!("ws://{}:{}/sdk/ws", 
                           cvm_info.ip_address, 
                           instance.metadata.sdk_port.unwrap_or(8080));

        let client = ChallengeWsClient::new(
            ws_url,
            instance.metadata.compose_hash.clone(),
            instance.metadata.challenge_id.clone(),
        );

        // Test WebSocket connection
        client.test_connection().await
            .context("Failed to establish WebSocket connection")?;

        Ok(client)
    }

    /// Run challenge via WebSocket communication
    async fn run_challenge_via_websocket(
        &self,
        client: ChallengeWsClient,
        instance: &ChallengeInstance,
    ) -> Result<()> {
        // Create callback for handling ORM queries
        let callback = self.create_orm_callback(&instance.metadata.compose_hash);

        // Connect and run challenge
        super::ws::connect_with_reconnect(&client, callback).await
            .context("WebSocket challenge execution failed")?;

        Ok(())
    }

    /// Create callback function for handling ORM queries
    fn create_orm_callback(&self, compose_hash: &str) -> impl Fn(Value, tokio::sync::mpsc::Sender<Value>) + Send + Sync + 'static {
        let compose_hash = compose_hash.to_string();
        let orm_gateway = self.orm_gateway.clone();
        let db_pool = self.db_pool.clone();

        move |query, response_tx| {
            let compose_hash = compose_hash.clone();
            let orm_gateway = orm_gateway.clone();
            let db_pool = db_pool.clone();

            tokio::spawn(async move {
                let response = if let Some(gateway) = orm_gateway {
                    // Use ORM gateway for query execution
                    handle_orm_query_via_gateway(query, &gateway, &compose_hash).await
                } else {
                    // Fallback to direct database execution
                    handle_orm_query_direct(query, &db_pool, &compose_hash).await
                };

                if let Err(e) = response_tx.send(response).await {
                    error!("Failed to send ORM query response: {}", e);
                }
            });
        }
    }

    /// Get challenge metadata by ID
    async fn get_challenge_metadata(&self, challenge_id: &str) -> Result<ChallengeMetadata> {
        let row = sqlx::query!(
            r#"
            SELECT c.id, c.compose_hash, c.name, c.description, c.spec,
                   c.created_at, c.updated_at, c.is_active
            FROM challenges c
            WHERE c.id = $1
            "#,
            challenge_id
        )
        .fetch_one(&self.db_pool)
        .await
        .context("Challenge not found")?;

        Ok(ChallengeMetadata {
            id: row.id,
            compose_hash: row.compose_hash,
            name: row.name,
            description: row.description,
            spec: row.spec,
            created_at: row.created_at,
            updated_at: row.updated_at,
            is_active: row.is_active,
            sdk_port: None,
        })
    }

    /// Get challenge by compose hash
    async fn get_challenge_by_compose_hash(&self, compose_hash: &str) -> Result<ChallengeMetadata> {
        let row = sqlx::query!(
            r#"
            SELECT c.id, c.compose_hash, c.name, c.description, c.spec,
                   c.created_at, c.updated_at, c.is_active
            FROM challenges c
            WHERE c.compose_hash = $1
            "#,
            compose_hash
        )
        .fetch_one(&self.db_pool)
        .await
        .context("Challenge not found")?;

        Ok(ChallengeMetadata {
            id: row.id,
            compose_hash: row.compose_hash,
            name: row.name,
            description: row.description,
            spec: row.spec,
            created_at: row.created_at,
            updated_at: row.updated_at,
            is_active: row.is_active,
            sdk_port: None,
        })
    }

    /// Get all running challenges
    pub async fn get_running_challenges(&self) -> Vec<String> {
        let challenges = self.challenges.read().await;
        challenges.keys().cloned().collect()
    }

    /// Check if challenge is running
    pub async fn is_challenge_running(&self, compose_hash: &str) -> bool {
        let challenges = self.challenges.read().await;
        challenges.contains_key(compose_hash)
    }

    /// Stop a running challenge
    pub async fn stop_challenge(&self, compose_hash: &str) -> Result<()> {
        info!("Stopping challenge: {}", compose_hash);

        // Remove from running challenges
        let instance = {
            let mut challenges = self.challenges.write().await;
            challenges.remove(compose_hash)
        };

        if let Some(instance) = instance {
            // Cleanup CVM if it was set up
            if let Some(cvm_info) = instance.cvm_info {
                if let Err(e) = self.cvm_manager.cleanup_cvm(&cvm_info).await {
                    warn!("Failed to cleanup CVM for challenge {}: {}", compose_hash, e);
                }
            }
        }

        info!("Challenge {} stopped", compose_hash);
        Ok(())
    }
}

/// Handle ORM query via gateway
async fn handle_orm_query_via_gateway(
    query: Value,
    gateway: &Arc<tokio::sync::RwLock<platform_api_orm_gateway::SecureORMGateway>>,
    compose_hash: &str,
) -> Value {
    let gateway = gateway.read().await;
    
    match gateway.execute_query(query, compose_hash).await {
        Ok(result) => result,
        Err(e) => {
            error!("ORM query via gateway failed: {}", e);
            serde_json::json!({
                "error": e.to_string(),
                "success": false
            })
        }
    }
}

/// Handle ORM query directly via database
async fn handle_orm_query_direct(
    query: Value,
    db_pool: &PgPool,
    _compose_hash: &str,
) -> Value {
    // This is a simplified implementation
    // In practice, you'd want to parse and execute the query safely
    match query.get("query_type").and_then(|t| t.as_str()) {
        Some("select") => {
            // Handle SELECT queries
            serde_json::json!({
                "success": true,
                "data": [],
                "message": "Direct query execution not implemented"
            })
        }
        _ => {
            error!("Unsupported query type for direct execution");
            serde_json::json!({
                "error": "Unsupported query type",
                "success": false
            })
        }
    }
}
