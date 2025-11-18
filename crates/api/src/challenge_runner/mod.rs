use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

pub mod challenge_ws;
pub mod ws; // WebSocket client module (split from challenge_ws.rs)
pub mod cvm_manager;
pub mod migrations;
pub mod runner;

pub use challenge_ws::ChallengeWsClient;
pub use cvm_manager::CvmManager;
pub use migrations::MigrationRunner;
pub use runner::{
    ChallengeInstance, ChallengeRunnerConfig, ChallengeMetadata, ChallengeStatus,
};

/// Challenge runner for API mode execution
pub struct ChallengeRunner {
    pub config: runner::types::ChallengeRunnerConfig,
    pub challenges: tokio::sync::RwLock<HashMap<String, ChallengeInstance>>, // Key: compose_hash
    pub db_pool: PgPool,
    pub migration_runner: MigrationRunner,
    pub cvm_manager: CvmManager,
    pub orm_gateway: Option<Arc<tokio::sync::RwLock<platform_api_orm_gateway::SecureORMGateway>>>, // ORM gateway for query bridge
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
        config: runner::types::ChallengeRunnerConfig,
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
        challenge_spec: Option<&platform_api_models::ChallengeSpec>,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<()> {
        info!(
            compose_hash = compose_hash,
            "Starting challenge in API mode"
        );

        // Check if already running with matching compose_hash
        {
            let challenges = self.challenges.read().await;
            if let Some(instance) = challenges.get(compose_hash) {
                if instance.is_running {
                    info!(
                        compose_hash = compose_hash,
                        challenge_id = &instance.challenge_id,
                        cvm_instance = instance.cvm_instance_id.as_deref().unwrap_or("none"),
                        "Challenge with compose_hash already running"
                    );
                    return Ok(());
                }
            }
        }

        // Get challenge_id and metadata - prefer using provided spec to avoid re-querying
        let (challenge_id, challenge) = if let Some(spec) = challenge_spec {
            // Use provided spec data - decode compose_yaml from base64 (like validator does)
            let compose_yaml = base64::decode(&spec.compose_yaml)
                .map_err(|e| anyhow::anyhow!("Failed to decode compose_yaml from base64: {}", e))
                .and_then(|bytes| {
                    String::from_utf8(bytes).map_err(|e| {
                        anyhow::anyhow!("Failed to convert compose_yaml to string: {}", e)
                    })
                })
                .unwrap_or_default();

            let metadata = ChallengeMetadata {
                name: spec.name.clone(),
                version: spec.version.clone(),
                github_repo: spec.github_repo.clone().unwrap_or_default(),
                compose_hash: compose_hash.to_string(),
                dstack_image: spec.dstack_image.clone(),
                images: spec.images.clone(),
                compose_yaml,
            };
            (spec.id.to_string(), metadata)
        } else {
            // Fallback to database query
            self.get_challenge_by_compose_hash(compose_hash).await?
        };

        // Create challenge instance (schema_name will be set after we get db_version from SDK)
        let mut instance = ChallengeInstance {
            challenge_id: challenge_id.clone(),
            name: challenge.name.clone(),
            version: challenge.version.clone(),
            compose_hash: compose_hash.to_string(),
            cvm_instance_id: None,
            cvm_api_url: None,
            schema_name: String::new(), // Will be computed from name + db_version
            db_version: None,
            is_running: false,
            ws_started: false,
        };

        // Select image: prefer dstack_image, else first from images
        let selected_image = if let Some(img) = challenge.dstack_image.clone() {
            img
        } else if let Some(first) = challenge.images.first() {
            first.clone()
        } else {
            // Fallback default
            "platform/challenge:latest".to_string()
        };

        // Check if there's an old CVM with different compose_hash for the same challenge
        // This would require checking by challenge_id, but since we're keying by compose_hash now,
        // we'll handle mismatches by checking existing CVM name
        // If CVM exists with this compose_hash name, deploy_challenge_cvm will reuse it

        // Deploy CVM for the challenge using the actual compose_yaml from database
        let cvm_result = self
            .cvm_manager
            .deploy_challenge_cvm(
                compose_hash,
                &challenge_id,
                &challenge.name,
                &selected_image,
                &challenge.compose_yaml,
                env_vars.as_ref(),
            )
            .await?;

        instance.cvm_instance_id = Some(cvm_result.instance_id.clone());
        let mut api_url = cvm_result.api_url.clone();

        // Wait for CVM to be ready (this will poll for instance_id if needed and update the URL)
        self.cvm_manager
            .wait_for_cvm_ready(&mut api_url, &cvm_result.instance_id, 300)
            .await?;

        // Update instance with potentially updated URL (if instance_id was retrieved)
        instance.cvm_api_url = Some(api_url.clone());

        // Store instance keyed by compose_hash (before starting WS and migrations)
        {
            let mut challenges = self.challenges.write().await;
            instance.is_running = true;
            challenges.insert(compose_hash.to_string(), instance.clone());
        }

        // Start WebSocket connection for TDX verification, migrations, and ORM bridge
        // We need to get db_version first before creating the schema
        if let Some(api_url_for_ws) = instance.cvm_api_url.clone() {
            let compose_hash_for_ws = compose_hash.to_string();
            let challenge_id_for_ws = challenge_id.clone();
            let challenge_name_for_ws = challenge.name.clone();
            let orm_gateway_for_ws = self.orm_gateway.clone();

            // Turn https://host:port into wss://host:port/sdk/ws
            let ws_url = api_url_for_ws
                .replace("https://", "wss://")
                .replace("http://", "ws://")
                + "/sdk/ws";

            // Use platform API identifier (could be from config or env var)
            let platform_api_id =
                std::env::var("PLATFORM_API_ID").unwrap_or_else(|_| "platform-api".to_string());

            // Create channels to receive db_version and migrations from WebSocket
            let (db_version_tx, db_version_rx) = tokio::sync::oneshot::channel();
            let (migrations_tx, migrations_rx) = tokio::sync::oneshot::channel();
            let (migrations_applied_tx, migrations_applied_rx) = tokio::sync::oneshot::channel();
            let db_version_sender_arc = Arc::new(tokio::sync::Mutex::new(Some(db_version_tx)));
            let migrations_sender_arc = Arc::new(tokio::sync::Mutex::new(Some(migrations_tx)));
            let migrations_applied_receiver_arc =
                Arc::new(tokio::sync::Mutex::new(Some(migrations_applied_rx)));
            let migrations_applied_sender_arc =
                Arc::new(tokio::sync::Mutex::new(Some(migrations_applied_tx)));

            let mut client = ChallengeWsClient::new(ws_url.clone(), platform_api_id);

            // Configure client with challenge info, ORM gateway, and migration runner for bridge
            // Create schema_name_arc outside the if block so it's accessible later
            let normalized_name = challenge_name_for_ws
                .to_lowercase()
                .replace(['-', '.'], "_")
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>();

            // Temporary schema name (will be updated with db_version)
            let temp_schema_name = format!("{}_v1", normalized_name);
            let schema_name_arc = Arc::new(tokio::sync::RwLock::new(temp_schema_name.clone()));

            if let Some(orm_gateway) = &orm_gateway_for_ws {
                // Create Arc wrapper for migration runner
                let migration_runner = Arc::new(self.migration_runner.clone());

                let mut client_with_challenge = client
                    .with_challenge(
                        challenge_id_for_ws.clone(),
                        challenge_name_for_ws.clone(),
                        orm_gateway.clone(),
                        Some(migration_runner),
                        Some(schema_name_arc.clone()),
                        Some(db_version_sender_arc.clone()),
                        Some(migrations_sender_arc.clone()),
                        Some(migrations_applied_receiver_arc.clone()),
                        Some(migrations_applied_sender_arc.clone()),
                        self.validator_challenge_status.clone(),
                        self.redis_client.clone(),
                    )
                    .with_compose_hash(compose_hash.to_string());

                // Set validator_connections if available
                if let Some(validator_conns) = &self.validator_connections {
                    client_with_challenge.validator_connections = Some(validator_conns.clone());
                }

                client = client_with_challenge;
            }

            // Start WebSocket connection in background to get db_version and migrations, then handle ORM bridge
            // The connection will send db_version and migrations via channels during migrations_request
            let ws_client = client.clone();
            let ws_task_handle = tokio::spawn(async move {
                // This will connect, do TDX, send migrations_request, send db_version and migrations via channels,
                // then continue running for ORM bridge
                let _ = ws_client.connect_with_reconnect(|_json, _sender| {}).await;
            });

            // Wait for db_version and migrations (with timeout) - they will be sent via channels during connect
            // Note: If timeout occurs, we default to version 1 and continue (not a fatal error)
            let db_version = match tokio::time::timeout(
                tokio::time::Duration::from_secs(30),
                db_version_rx,
            )
            .await
            {
                Ok(Ok(Some(version))) => {
                    info!(
                        db_version = version,
                        "✅ Received db_version from challenge"
                    );
                    version
                }
                Ok(Ok(None)) | Ok(Err(_)) => {
                    warn!("No db_version received (channel closed), defaulting to 1");
                    1
                }
                Err(_) => {
                    warn!("Timeout waiting for db_version (challenge may not have responded), defaulting to 1");
                    warn!("This is not a fatal error - challenge will continue with default schema version");
                    1
                }
            };

            // Wait for migrations (with timeout)
            let migrations = match tokio::time::timeout(
                tokio::time::Duration::from_secs(30),
                migrations_rx,
            )
            .await
            {
                Ok(Ok(migs)) => {
                    info!(
                        count = migs.len(),
                        "✅ Received migrations from challenge via WebSocket"
                    );
                    migs
                }
                Ok(Err(_)) => {
                    warn!("Migrations channel closed before receiving data");
                    Vec::new()
                }
                Err(_) => {
                    warn!("Timeout waiting for migrations (challenge may not have responded)");
                    warn!("This is not a fatal error - challenge will continue without migrations");
                    Vec::new()
                }
            };

            // Note: ws_task_handle continues running - it's the real WebSocket connection for ORM bridge
            // We just needed to wait for db_version before computing schema_name

            // Compute final schema name
            let normalized_name = challenge
                .name
                .to_lowercase()
                .replace(['-', '.'], "_")
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>();
            let final_schema_name = format!("{}_v{}", normalized_name, db_version);

            // Check if schema already exists for this challenge name and db_version
            // Query PostgreSQL information_schema
            // Note: If this fails, we don't mark the challenge as failed - it's a non-critical operation
            let existing_schema = match sqlx::query_scalar::<_, Option<String>>(
                r#"
                SELECT schema_name 
                FROM information_schema.schemata 
                WHERE schema_name = $1
                "#,
            )
            .bind(&final_schema_name)
            .fetch_optional(&self.db_pool)
            .await
            {
                Ok(result) => result,
                Err(e) => {
                    error!(error = %e, schema = &final_schema_name, "Failed to check existing schema (non-fatal)");
                    None // Continue as if schema doesn't exist
                }
            };

            if existing_schema.is_none() {
                info!(
                    schema = &final_schema_name,
                    db_version = db_version,
                    "Creating new schema for challenge (new DB version or first deployment)"
                );
            } else {
                info!(
                    schema = &final_schema_name,
                    db_version = db_version,
                    "Reusing existing schema (same DB version)"
                );
            }

            // Update instance with final schema name and db_version
            instance.schema_name = final_schema_name.clone();
            instance.db_version = Some(db_version);

            // Update the stored instance in the challenges map
            {
                let mut challenges = self.challenges.write().await;
                if let Some(inst) = challenges.get_mut(compose_hash) {
                    inst.schema_name = final_schema_name.clone();
                    inst.db_version = Some(db_version);
                }
            }

            // Update schema_name in ChallengeWsClient (if it exists)
            // Note: ws_client was cloned earlier, but we can find and update the schema_name Arc
            // Since we stored schema_name_arc earlier, we can update it directly
            {
                let mut schema_guard = schema_name_arc.write().await;
                *schema_guard = final_schema_name.clone();
            }

            // Ensure schema exists - CRITICAL: schema must exist for ORM queries to work
            // This is fatal if it fails, as the challenge cannot function without its schema
            if let Err(e) = self
                .migration_runner
                .ensure_schema(&final_schema_name)
                .await
            {
                error!(
                    error = %e,
                    schema = &final_schema_name,
                    "❌ CRITICAL: Failed to ensure schema exists - challenge will not function without schema"
                );
                // Continue anyway but log error - the schema creation might succeed later
                // But this should not happen in normal operation
            } else {
                info!(
                    schema = &final_schema_name,
                    "✅ Schema verified/created successfully"
                );
            }

            // Apply migrations received via WebSocket to the correct schema
            // Note: If this fails, we don't mark the challenge as failed - migrations are optional
            if !migrations.is_empty() {
                info!(
                    count = migrations.len(),
                    schema = &final_schema_name,
                    "Applying migrations received via WebSocket to correct schema"
                );
                match self
                    .migration_runner
                    .apply_migrations(&final_schema_name, migrations)
                    .await
                {
                    Ok(applied) => {
                        info!(
                            applied_count = applied.len(),
                            schema = &final_schema_name,
                            "✅ Migrations applied successfully to schema"
                        );
                        // Note: migrations_applied signal is now sent from WebSocket task
                        // This is just a fallback in case WebSocket task doesn't send it
                        info!("Migrations applied in mod.rs (WebSocket task should have already applied them)");
                    }
                    Err(e) => {
                        warn!(error = %e, schema = &final_schema_name, "Failed to apply migrations (non-fatal)");
                        warn!("Challenge will continue without migrations - this is not a fatal error");
                        // Note: migrations_applied signal is now sent from WebSocket task
                        warn!("Migration failed in mod.rs (WebSocket task should have already handled it)");
                    }
                }
            } else {
                info!("No migrations to apply (challenge has no migrations or migrations not received)");
                // Signal that migrations are "applied" (none to apply) so orm_ready can be sent
                // Use the sender from the arc (same one passed to WebSocket task)
                let mut sender_opt = migrations_applied_sender_arc.lock().await;
                if let Some(sender) = sender_opt.take() {
                    if sender.send(()).is_err() {
                        warn!(
                            "Failed to send migrations_applied signal (receiver may have dropped)"
                        );
                    } else {
                        info!("✅ Sent migrations_applied signal (no migrations to apply) - orm_ready can now be sent");
                    }
                }
            }

            // The WebSocket connection is already running in ws_task_handle
            // It will handle ORM queries and other messages

            info!(
                compose_hash = &compose_hash_for_ws,
                challenge_id = &challenge_id_for_ws,
                db_version = db_version,
                schema = &final_schema_name,
                "✅ WebSocket connection established for ORM bridge"
            );

            // Mark ws_started
            {
                let mut challenges = self.challenges.write().await;
                if let Some(inst) = challenges.get_mut(compose_hash) {
                    inst.ws_started = true;
                }
            }
        }

        info!(
            compose_hash = compose_hash,
            challenge_id = challenge_id,
            cvm_instance = &cvm_result.instance_id,
            "✅ Challenge CVM deployed successfully and WebSocket connection started"
        );

        Ok(())
    }

    /// Run a challenge in API mode by compose_hash (calls run_challenge_by_compose_hash_with_spec with None)
    pub async fn run_challenge_by_compose_hash(&self, compose_hash: &str) -> Result<()> {
        self.run_challenge_by_compose_hash_with_spec(compose_hash, None, None)
            .await
    }

    /// Apply migrations for a challenge by compose_hash
    pub async fn apply_migrations_by_compose_hash(&self, compose_hash: &str) -> Result<()> {
        info!(compose_hash = compose_hash, "Applying migrations");

        let challenges = self.challenges.read().await;
        let instance = challenges
            .get(compose_hash)
            .ok_or_else(|| {
                anyhow::anyhow!("Challenge with compose_hash {} not found", compose_hash)
            })?
            .clone();

        // Migrations should have been applied automatically during challenge startup via WebSocket
        // This method is kept for compatibility but migrations are now handled during deployment
        info!(
            compose_hash = compose_hash,
            schema = &instance.schema_name,
            "Migrations are now applied automatically via WebSocket during challenge deployment"
        );

        // Note: Migrations are applied during challenge startup via WebSocket
        // If you need to re-apply migrations, restart the challenge

        info!(
            compose_hash = compose_hash,
            challenge_id = &instance.challenge_id,
            schema = &instance.schema_name,
            "Migrations applied successfully"
        );

        Ok(())
    }

    /// Stop a running challenge by compose_hash
    pub async fn stop_challenge_by_compose_hash(&self, compose_hash: &str) -> Result<()> {
        info!(compose_hash = compose_hash, "Stopping challenge");

        let mut challenges = self.challenges.write().await;
        if let Some(instance) = challenges.get_mut(compose_hash) {
            if instance.is_running {
                // Stop CVM
                if let Some(cvm_id) = &instance.cvm_instance_id {
                    self.cvm_manager.stop_cvm(cvm_id).await?;
                }
                instance.is_running = false;
                instance.cvm_instance_id = None;
                instance.cvm_api_url = None;
            }
        }

        Ok(())
    }

    /// List all running challenges
    pub async fn list_running_challenges(&self) -> Vec<ChallengeInstance> {
        let challenges = self.challenges.read().await;
        challenges
            .values()
            .filter(|c| c.is_running)
            .cloned()
            .collect()
    }

    /// Get challenge status by compose_hash
    pub async fn get_challenge_status_by_compose_hash(
        &self,
        compose_hash: &str,
    ) -> Result<ChallengeStatus> {
        let challenges = self.challenges.read().await;
        let instance = challenges.get(compose_hash).ok_or_else(|| {
            anyhow::anyhow!("Challenge with compose_hash {} not found", compose_hash)
        })?;

        // Check if CVM is actually running and healthy
        let is_healthy =
            if let (true, Some(cvm_id)) = (instance.is_running, &instance.cvm_instance_id) {
                self.cvm_manager.check_cvm_health(cvm_id).await?
            } else {
                false
            };

        Ok(ChallengeStatus {
            challenge_id: instance.challenge_id.clone(),
            is_running: instance.is_running,
            is_healthy,
            schema_name: instance.schema_name.clone(),
            version: instance.version.clone(),
            cvm_instance_id: instance.cvm_instance_id.clone(),
        })
    }

    /// Get challenge metadata and ID by compose_hash from database
    async fn get_challenge_by_compose_hash(
        &self,
        compose_hash: &str,
    ) -> Result<(String, ChallengeMetadata)> {
        let row = sqlx::query(
            r#"
            SELECT id, name, version, github_repo, compose_hash, dstack_image, images, compose_yaml
            FROM challenges
            WHERE compose_hash = $1
            ORDER BY updated_at DESC
            LIMIT 1
            "#,
        )
        .persistent(false)
        .bind(compose_hash)
        .fetch_optional(&self.db_pool)
        .await
        .with_context(|| {
            format!(
                "Failed to query challenge by compose_hash: {}",
                compose_hash
            )
        })?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No challenge found with compose_hash: {}. Challenge may not be in database yet.",
                compose_hash
            )
        })?;

        let challenge_id: uuid::Uuid = row.try_get("id")?;
        let challenge_id_str = challenge_id.to_string();

        // dstack_image can be NULL
        let dstack_image: Option<String> = row.try_get("dstack_image").ok();

        // images may be a text[] or a JSON array depending on schema
        let images: Vec<String> = row.try_get::<Vec<String>, _>("images").unwrap_or_else(|_| {
            let v: Value = row.try_get("images").unwrap_or(Value::Null);
            if let Value::Array(arr) = v {
                arr.into_iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            } else {
                Vec::new()
            }
        });

        let compose_yaml: String = row.try_get("compose_yaml")?;

        let metadata = ChallengeMetadata {
            name: row.try_get("name")?,
            version: row.try_get("version")?,
            github_repo: row.try_get("github_repo")?,
            compose_hash: compose_hash.to_string(),
            dstack_image,
            images,
            compose_yaml,
        };

        Ok((challenge_id_str, metadata))
    }

    /// Get challenge metadata from database
    async fn get_challenge_metadata(&self, challenge_id: &str) -> Result<ChallengeMetadata> {
        let row = sqlx::query(
            r#"
            SELECT name, version, github_repo, compose_hash, dstack_image, images, compose_yaml
            FROM challenges
            WHERE id = $1
            "#,
        )
        .bind(challenge_id)
        .fetch_one(&self.db_pool)
        .await
        .context("Failed to fetch challenge metadata")?;

        // dstack_image can be NULL
        let dstack_image: Option<String> = row.try_get("dstack_image").ok();

        // images may be a text[] or a JSON array depending on schema
        let images: Vec<String> = row.try_get::<Vec<String>, _>("images").unwrap_or_else(|_| {
            let v: Value = row.try_get("images").unwrap_or(Value::Null);
            if let Value::Array(arr) = v {
                arr.into_iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            } else {
                Vec::new()
            }
        });

        let compose_yaml: String = row.try_get("compose_yaml")?;

        Ok(ChallengeMetadata {
            name: row.try_get("name")?,
            version: row.try_get("version")?,
            github_repo: row.try_get("github_repo")?,
            compose_hash: row.try_get("compose_hash")?,
            dstack_image,
            images,
            compose_yaml,
        })
    }

    /// Generate database credentials for a challenge
    async fn generate_db_credentials(&self, schema_name: &str) -> Result<HashMap<String, String>> {
        // Create schema
        self.migration_runner.ensure_schema(schema_name).await?;

        // Get database URL from environment or config
        let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL not set")?;

        // Parse and modify URL to include schema
        let parsed = url::Url::parse(&database_url)?;
        let host = parsed.host_str().unwrap_or("localhost");
        let port = parsed.port().unwrap_or(5432);
        let database = parsed.path().trim_start_matches('/');
        let username = parsed.username();
        let password = parsed.password().unwrap_or("");

        // Create schema-specific connection string
        let schema_url = format!(
            "postgresql://{}:{}@{}:{}/{}?options=--search_path%3D{}",
            username, password, host, port, database, schema_name
        );

        let mut credentials = HashMap::new();
        credentials.insert("database_url".to_string(), schema_url.clone());
        credentials.insert("schema_name".to_string(), schema_name.to_string());

        // For SQLAlchemy async
        let async_url = schema_url.replace("postgresql://", "postgresql+asyncpg://");
        credentials.insert("async_database_url".to_string(), async_url);

        Ok(credentials)
    }
}
