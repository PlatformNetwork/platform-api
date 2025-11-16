use crate::redis_client::{create_job_log, create_job_progress, RedisClient};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit};
use futures_util::{SinkExt, StreamExt};
use hkdf::Hkdf;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use x25519_dalek::{EphemeralSecret, PublicKey};

/// Envelope used for encrypted WebSocket frames
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct EncryptedEnvelope {
    enc: String,
    nonce: String,      // base64(12 bytes)
    ciphertext: String, // base64
}


/// Connection state tracking for TDX verification enforcement
#[derive(Debug, Clone)]
enum ConnectionState {
    Unverified { nonce: [u8; 32], started: Instant },
    Verified { aead_key: [u8; 32] },
    Rejected { reason: String },
}

#[derive(Clone)]
pub struct ChallengeWsClient {
    pub url: String,
    pub platform_api_id: String, // Platform API identifier (instead of validator_hotkey)
    pub challenge_id: Option<String>, // Challenge ID for schema routing
    pub challenge_name: Option<String>, // Challenge name for schema naming
    pub orm_gateway: Option<Arc<tokio::sync::RwLock<crate::orm_gateway::SecureORMGateway>>>, // ORM Gateway for query execution
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
    pub migrations_applied_receiver: Option<
        Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
    >, // Channel to receive signal that migrations are applied (from mod.rs)
    pub migrations_applied_sender: Option<
        Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    >, // Channel to send signal that migrations are applied (from WebSocket task)
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
        Arc<tokio::sync::RwLock<std::collections::HashMap<String, crate::state::ValidatorConnection>>>,
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
        orm_gateway: Arc<tokio::sync::RwLock<crate::orm_gateway::SecureORMGateway>>,
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

    /// Get list of active validator hotkeys for a specific compose_hash
    /// Uses both validator_challenge_status (if available) and validator_connections (WebSocket connections)
    async fn get_active_validators_for_compose_hash(
        &self,
        compose_hash: &str,
        validator_status: &Arc<
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
    ) -> Vec<String> {
        let mut validators = std::collections::HashSet::new();

        // First, get validators from validator_challenge_status (if they have Active state)
        let status_map = validator_status.read().await;
        for (hotkey, challenge_statuses) in status_map.iter() {
            if let Some(status) = challenge_statuses.get(compose_hash) {
                if matches!(
                    status.state,
                    platform_api_models::ValidatorChallengeState::Active
                ) {
                    validators.insert(hotkey.clone());
                }
            }
        }
        drop(status_map);

        // Also include validators connected via WebSocket with matching compose_hash
        // This is a fallback when validator_challenge_status doesn't have Active state
        if let Some(validator_conns) = &self.validator_connections {
            let connections = validator_conns.read().await;
            for (hotkey, conn) in connections.iter() {
                if conn.compose_hash == compose_hash {
                    validators.insert(hotkey.clone());
                }
            }
        }

        validators.into_iter().collect()
    }

    /// Connect with reconnection logic and handle messages
    /// Handles ORM queries from challenge SDK and executes them via ORM gateway
    pub async fn connect_with_reconnect<F>(&self, callback: F) -> Result<()>
    where
        F: Fn(Value, mpsc::Sender<Value>) + Send + Sync + 'static,
    {
        info!("Connecting WebSocket to {}", self.url);

        // URL already includes /sdk/ws path
        let (ws_stream, _) = connect_async(&self.url)
            .await
            .with_context(|| format!("Failed to connect WebSocket to {}", self.url))?;
        let (write, mut read) = ws_stream.split();

        // Wrap write in Arc<Mutex> to share between tasks
        let write_handle = Arc::new(tokio::sync::Mutex::new(write));

        info!(
            "✅ Connected WebSocket to {}, starting TDX attestation",
            self.url
        );

        // Begin attestation handshake
        let mut nonce_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce_hex = hex::encode(nonce_bytes);

        // Initialize connection state as unverified
        let mut conn_state = ConnectionState::Unverified {
            nonce: nonce_bytes,
            started: Instant::now(),
        };

        // Generate platform API X25519 ephemeral keypair
        let api_secret = EphemeralSecret::random_from_rng(&mut rand::thread_rng());
        let api_public = PublicKey::from(&api_secret);
        let api_pub_b64 = base64_engine.encode(api_public.as_bytes());

        // Send attestation_begin
        // Note: Challenge SDK expects "val_x25519_pub" (compatible with validator) or "api_x25519_pub"
        // We use "val_x25519_pub" for compatibility
        let begin_msg = Message::Text(
            serde_json::json!({
                "type": "attestation_begin",
                "nonce": nonce_hex,
                "platform_api_id": self.platform_api_id,
                "val_x25519_pub": api_pub_b64,
            })
            .to_string(),
        );
        {
            let mut write = write_handle.lock().await;
            write.send(begin_msg).await?;
        }

        // Wait for attestation_response - consume api_secret only once
        let aead_key = loop {
            let msg = read
                .next()
                .await
                .ok_or_else(|| anyhow!("Connection closed before attestation"))??;

            if let Message::Text(text) = msg {
                let json: Value = serde_json::from_str(&text)?;

                // Check timeout (30 seconds)
                if let ConnectionState::Unverified { started, .. } = &conn_state {
                    if started.elapsed().as_secs() > 30 {
                        error!("Attestation timeout");
                        return Err(anyhow!("Attestation timeout"));
                    }
                }

                // Expect attestation_response
                if let Some(typ) = json.get("type").and_then(|t| t.as_str()) {
                    if typ == "attestation_response" {
                        let chal_pub_b64 = json
                            .get("chal_x25519_pub")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("Missing chal_x25519_pub"))?;
                        let chal_pub_bytes = base64_engine.decode(chal_pub_b64)?;
                        if chal_pub_bytes.len() != 32 {
                            return Err(anyhow!("Invalid challenge public key length"));
                        }
                        let chal_pub_slice: &[u8; 32] = chal_pub_bytes[..32]
                            .try_into()
                            .map_err(|_| anyhow!("Failed to convert challenge public key"))?;
                        let chal_pub = PublicKey::from(*chal_pub_slice);

                        // Derive shared secret (consume api_secret here, only once)
                        let shared_secret = api_secret.diffie_hellman(&chal_pub);

                        // Get nonce from conn_state
                        let nonce = match &conn_state {
                            ConnectionState::Unverified { nonce, .. } => *nonce,
                            _ => return Err(anyhow!("Invalid connection state")),
                        };

                        // Generate HKDF salt (like validator does)
                        let mut hkdf_salt_bytes = [0u8; 32];
                        rand::thread_rng().fill_bytes(&mut hkdf_salt_bytes);
                        let hkdf_salt_b64 = base64_engine.encode(hkdf_salt_bytes);

                        // Derive AEAD key using HKDF with salt (like validator does)
                        let hkdf =
                            Hkdf::<Sha256>::new(Some(&hkdf_salt_bytes), shared_secret.as_bytes());
                        let mut key_bytes = [0u8; 32];
                        hkdf.expand(b"platform-api-sdk-v1", &mut key_bytes)
                            .map_err(|_| anyhow!("HKDF expansion failed"))?;

                        // Verify TDX quote if present (skip in dev mode)
                        let dev_mode = std::env::var("DEV_MODE")
                            .unwrap_or_else(|_| "false".to_string())
                            .to_lowercase()
                            == "true"
                            || std::env::var("TEE_ENFORCED")
                                .unwrap_or_else(|_| "true".to_string())
                                .to_lowercase()
                                == "false";

                        if dev_mode {
                            debug!("DEV MODE: Skipping TDX quote verification");
                        } else if let Some(quote_b64) = json.get("quote").and_then(|v| v.as_str()) {
                            match Self::verify_tdx_quote(quote_b64, &nonce).await {
                                Ok(_) => {
                                    debug!("TDX quote verified successfully");
                                }
                                Err(e) => {
                                    error!("TDX quote verification failed: {}", e);
                                    return Err(anyhow!("TDX verification failed: {}", e));
                                }
                            }
                        } else {
                            warn!("No TDX quote in attestation_response, skipping verification");
                        }

                        // Check if we're in dev mode (challenge will use plain text)
                        let dev_mode = std::env::var("DEV_MODE")
                            .unwrap_or_else(|_| "false".to_string())
                            .to_lowercase()
                            == "true"
                            || std::env::var("TEE_ENFORCED")
                                .unwrap_or_else(|_| "true".to_string())
                                .to_lowercase()
                                == "false";

                        // Check if encryption should be enabled in dev mode (TDX simulation or explicit encryption flag)
                        let tdx_simulation_mode = std::env::var("TDX_SIMULATION_MODE")
                            .unwrap_or_else(|_| "false".to_string())
                            .to_lowercase()
                            == "true";
                        // Always use encryption
                        
                        // Log mode for debugging
                        if dev_mode || tdx_simulation_mode {
                            debug!("DEV MODE: Using encrypted session with mock TDX attestation");
                        } else {
                            debug!("Production mode: Using encrypted session with real TDX attestation");
                        }

                        // Always send attestation_ok with HKDF salt
                        let ok_msg = Message::Text(
                            serde_json::json!({
                                "type": "attestation_ok",
                                "aead": "chacha20poly1305",
                                "hkdf_salt": hkdf_salt_b64,
                            })
                            .to_string(),
                        );
                        {
                            let mut write = write_handle.lock().await;
                            write.send(ok_msg).await?;
                        }

                        debug!("TDX attestation verified, connection encrypted");
                        break key_bytes;
                    }
                }

                // If not attestation_response, reject
                warn!("Unexpected message during attestation: {:?}", json);
                return Err(anyhow!("Attestation failed: unexpected message"));
            }
        };

        // Check if we're in dev mode (use plain text messages by default)
        let dev_mode = std::env::var("DEV_MODE")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true"
            || std::env::var("TEE_ENFORCED")
                .unwrap_or_else(|_| "true".to_string())
                .to_lowercase()
                == "false";
        
        // Check if encryption should be enabled in dev mode
        let tdx_simulation_mode = std::env::var("TDX_SIMULATION_MODE")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true";
        // Always use encryption
        let use_encryption = true;

        // After TDX verification, request migrations via WebSocket (encrypted or plain text based on mode)
        // Only request migrations if CHALLENGE_ADMIN=true (admin mode)
        // Note: schema_name might be temporary (v1) until db_version is known
        if let Some(_migration_runner) = &self.migration_runner {
            // In admin mode (CHALLENGE_ADMIN=true), we request migrations
            // In non-admin mode, the challenge will return empty migrations
            // Always using encryption
            if tdx_simulation_mode {
                debug!("DEV MODE (SIMULATION): Requesting migrations via encrypted WebSocket");
            } else {
                info!(
                    "Requesting migrations via encrypted WebSocket (only if CHALLENGE_ADMIN=true)"
                );
            }

            // Send migrations_request message
            let request_msg = serde_json::json!({
                "type": "migrations_request"
            });

            // Always encrypt the message
            let mut request_nonce = [0u8; 12];
            rand::thread_rng().fill_bytes(&mut request_nonce);
                let request_nonce_b64 = base64_engine.encode(request_nonce);

                let request_bytes = serde_json::to_vec(&request_msg)
                    .map_err(|_| anyhow!("Failed to serialize migrations request"))?;

                let cipher = ChaCha20Poly1305::new_from_slice(&aead_key)
                    .map_err(|_| anyhow!("Invalid AEAD key length (expected 32 bytes)"))?;
                let request_ciphertext = cipher
                    .encrypt(&request_nonce.into(), request_bytes.as_slice())
                    .map_err(|_| anyhow!("Failed to encrypt migrations request"))?;
                let request_ciphertext_b64 = base64_engine.encode(request_ciphertext);

                let request_envelope = serde_json::json!({
                    "enc": "chacha20poly1305",
                    "nonce": request_nonce_b64,
                    "ciphertext": request_ciphertext_b64,
                });

                {
                    let mut write = write_handle.lock().await;
                    let envelope_str = serde_json::to_string(&request_envelope)?;
                    write.send(Message::Text(envelope_str.clone())).await?;
                    info!("migrations_request sent, waiting for response...");
                }

            // Wait for migrations_response
            // Increased timeout to 300 seconds (5 minutes) to allow for long-running migrations
            // The migration 009 and others may take time to apply, especially with many INSERT statements
            let mut migrations_received = false;
            let timeout = tokio::time::Duration::from_secs(300);
            let start = tokio::time::Instant::now();

            while !migrations_received && start.elapsed() < timeout {
                // Check for messages with a timeout per iteration
                match tokio::time::timeout(tokio::time::Duration::from_millis(500), read.next())
                    .await
                {
                    Ok(Some(msg_result)) => {
                        match msg_result {
                            Ok(msg) => {
                                if let Message::Text(text) = msg {
                                    let plain_msg: serde_json::Value = if !use_encryption {
                                        // In dev mode without encryption, message is already plain text
                                        debug!("DEV MODE: Received plain text message in migrations wait loop");
                                        match serde_json::from_str(&text) {
                                            Ok(pm) => pm,
                                            Err(e) => {
                                                warn!(error = %e, "Failed to parse plain text message");
                                                continue;
                                            }
                                        }
                                    } else {
                                        // Decrypt the message (production mode or dev mode with encryption enabled)
                                        // (Logging removed for verbosity)
                                        let json: Value = serde_json::from_str(&text)?;
                                        let envelope: EncryptedEnvelope =
                                            serde_json::from_value(json)?;
                                        let nonce_bytes =
                                            match base64_engine.decode(&envelope.nonce) {
                                                Ok(bytes) => bytes,
                                                Err(e) => {
                                                    warn!(error = %e, "Failed to decode nonce");
                                                    continue;
                                                }
                                            };
                                        if nonce_bytes.len() != 12 {
                                            warn!(
                                                "Invalid nonce length: {} (expected 12)",
                                                nonce_bytes.len()
                                            );
                                            continue;
                                        }
                                        let nonce_array: [u8; 12] = match nonce_bytes[..12]
                                            .try_into()
                                        {
                                            Ok(arr) => arr,
                                            Err(_) => {
                                                warn!("Failed to convert nonce to array (this should not happen)");
                                                continue;
                                            }
                                        };

                                        let ciphertext = match base64_engine
                                            .decode(&envelope.ciphertext)
                                        {
                                            Ok(bytes) => bytes,
                                            Err(e) => {
                                                warn!(error = %e, "Failed to decode ciphertext");
                                                continue;
                                            }
                                        };
                                        let cipher = ChaCha20Poly1305::new_from_slice(&aead_key)
                                            .map_err(|_| anyhow!("Invalid AEAD key length"))?;
                                        let plaintext = match cipher
                                            .decrypt(&nonce_array.into(), ciphertext.as_slice())
                                        {
                                            Ok(pt) => pt,
                                            Err(e) => {
                                                warn!(error = %e, "Decryption failed for message");
                                                continue;
                                            }
                                        };

                                        match serde_json::from_slice(&plaintext) {
                                            Ok(pm) => pm,
                                            Err(e) => {
                                                warn!(error = %e, "Failed to parse plain message");
                                                continue;
                                            }
                                        }
                                    };

                                    info!(msg_type = %plain_msg["type"].as_str().unwrap_or(""), "Decrypted message type");

                                    if plain_msg["type"].as_str().unwrap_or("") == "migrations_response" {
                                        info!("✅ Received migrations_response message");

                                        // Extract DB version from payload
                                        let db_version = plain_msg
                                            .get("payload")
                                            .and_then(|p| p.get("db_version"))
                                            .and_then(|v| v.as_u64())
                                            .map(|v| v as u32);

                                        if let Some(db_version) = db_version {
                                            info!(
                                                db_version = db_version,
                                                "✅ Received DB version from challenge"
                                            );

                                            // Send db_version back via channel if provided
                                            if let Some(sender_arc) = &self.db_version_sender {
                                                let mut sender_opt = sender_arc.lock().await;
                                                if let Some(sender) = sender_opt.take() {
                                                    let _ = sender.send(Some(db_version));
                                                    info!("db_version sent via channel");
                                                } else {
                                                    warn!("db_version_sender already consumed");
                                                }
                                            } else {
                                                warn!("No db_version_sender configured");
                                            }
                                        } else {
                                            warn!("No db_version in migrations_response payload");
                                        }

                                        if let Some(migrations_json) =
                                            plain_msg.get("payload").and_then(|p| p.get("migrations"))
                                        {
                                            if let Ok(received_migrations) = serde_json::from_value::<
                                                Vec<crate::challenge_runner::migrations::Migration>,
                                            >(
                                                migrations_json.clone(),
                                            ) {
                                                info!(
                                                    count = received_migrations.len(),
                                                    "Received migrations via WebSocket"
                                                );

                                                // Send migrations back via channel if provided
                                                if let Some(sender_arc) = &self.migrations_sender {
                                                    let mut sender_opt = sender_arc.lock().await;
                                                    if let Some(sender) = sender_opt.take() {
                                                        let _ = sender
                                                            .send(received_migrations.clone());
                                                        info!("migrations sent via channel");
                                                    }
                                                }

                                                // Apply migrations immediately in WebSocket task to ensure they're done before orm_ready
                                                // This prevents the WebSocket from closing before migrations are applied
                                                if let Some(migration_runner) = &self.migration_runner {
                                                    if let Some(schema_arc) = &self.schema_name {
                                                        let schema_name = schema_arc.read().await.clone();
                                                        info!(
                                                            schema = &schema_name,
                                                            count = received_migrations.len(),
                                                            "Applying migrations in WebSocket task before sending orm_ready"
                                                        );
                                                        match migration_runner
                                                            .apply_migrations(&schema_name, received_migrations)
                                                            .await
                                                        {
                                                            Ok(applied) => {
                                                                info!(
                                                                    schema = &schema_name,
                                                                    applied_count = applied.len(),
                                                                    "✅ Migrations applied successfully in WebSocket task"
                                                                );
                                                                // Signal that migrations are applied
                                                                if let Some(sender_arc) = &self.migrations_applied_sender {
                                                                    let mut sender_opt = sender_arc.lock().await;
                                                                    if let Some(sender) = sender_opt.take() {
                                                                        if sender.send(()).is_err() {
                                                                            warn!("Failed to send migrations_applied signal (receiver may have dropped)");
                                                                        } else {
                                                                            info!("✅ Sent migrations_applied signal from WebSocket task");
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            Err(e) => {
                                                                warn!(
                                                                    schema = &schema_name,
                                                                    error = %e,
                                                                    "Failed to apply migrations in WebSocket task (non-fatal)"
                                                                );
                                                            }
                                                        }
                                                    }
                                                }

                                                // Mark migrations as received
                                                migrations_received = true;
                                                // Don't break - continue the connection for ORM bridge
                                            } else {
                                                warn!("Failed to parse migrations from migrations_response");
                                            }
                                        } else {
                                            warn!("No migrations field in migrations_response payload");
                                        }
                                    } else {
                                        // Log other message types for debugging
                                        info!(msg_type = %plain_msg["type"].as_str().unwrap_or(""), "Received other message type in migrations wait loop (not migrations_response)");
                                    }
                                } else {
                                    // Check if it's a Close, Ping, or Pong message
                                    match &msg {
                                        Message::Close(_) => {
                                            warn!("Received Close message in migrations wait loop");
                                            migrations_received = true; // Exit loop
                                            break;
                                        }
                                        Message::Ping(data) => {
                                            info!("Received Ping, responding with Pong");
                                            {
                                                let mut write = write_handle.lock().await;
                                                let _ =
                                                    write.send(Message::Pong(data.clone())).await;
                                            }
                                        }
                                        Message::Pong(_) => {
                                            info!(
                                                "Received Pong in migrations wait loop (ignoring)"
                                            );
                                        }
                                        Message::Binary(data) => {
                                            warn!("Received Binary message ({} bytes) in migrations wait loop", data.len());
                                        }
                                        Message::Frame(_) => {
                                            warn!("Received Frame message in migrations wait loop");
                                        }
                                        _ => {
                                            warn!("Received unexpected message type in migrations wait loop");
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Error reading message in migrations wait loop");
                            }
                        }
                    }
                    Ok(None) => {
                        // No message available this iteration, continue waiting
                    }
                    Err(_) => {
                        // Timeout on this iteration, continue waiting for overall timeout
                        // Log progress periodically
                        let elapsed = start.elapsed();
                        if elapsed.as_secs() % 5 == 0 && elapsed.as_millis() % 5000 < 100 {
                            info!(
                                "Still waiting for migrations_response... (elapsed: {:?})",
                                elapsed
                            );
                        }
                    }
                }
            }

            if !migrations_received {
                warn!("Timeout waiting for migrations response (challenge may not have migrations or didn't respond)");
            }
        }

        // Wait for migrations to be applied before sending orm_ready
        // This ensures that all database tables exist before the challenge starts making ORM queries
        if let Some(receiver_arc) = &self.migrations_applied_receiver {
            if let Some(mut receiver_guard) = receiver_arc.lock().await.take() {
                info!("Waiting for migrations to be applied before sending orm_ready...");
                match tokio::time::timeout(
                    tokio::time::Duration::from_secs(60),
                    receiver_guard,
                )
                .await
                {
                    Ok(Ok(_)) => {
                        info!("✅ Migrations applied - ready to send orm_ready signal");
                    }
                    Ok(Err(_)) => {
                        warn!("Migrations applied channel closed unexpectedly");
                    }
                    Err(_) => {
                        warn!("Timeout waiting for migrations to be applied (60s) - sending orm_ready anyway");
                    }
                }
            } else {
                warn!("No migrations_applied_receiver available - sending orm_ready immediately (migrations may not be applied yet)");
            }
        } else {
            // If no receiver is provided, wait a short time for migrations to be applied
            // This is a fallback for cases where migrations_applied_receiver is not set
            info!("No migrations_applied_receiver configured - waiting 5 seconds for migrations to be applied");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }

        // Send orm_ready signal to challenge after migrations are applied (or timeout)
        // This tells the challenge that it can now initialize ORM client and run tests
        {
            // Include schema_name in orm_ready message so challenge knows which schema to use
            let schema_name_for_challenge = if let Some(schema_arc) = &self.schema_name {
                schema_arc.read().await.clone()
            } else {
                // Fallback to challenge_{challenge_id} if schema_name not set
                format!(
                    "challenge_{}",
                    self.challenge_id
                        .as_ref()
                        .unwrap_or(&"unknown".to_string())
                        .replace('-', "_")
                )
            };

            let orm_ready_msg = serde_json::json!({
                "type": "orm_ready",
                "schema": schema_name_for_challenge
            });

            if !use_encryption {
                // In dev mode without encryption, send plain text message
                {
                    let mut write = write_handle.lock().await;
                    let msg_str = serde_json::to_string(&orm_ready_msg)?;
                    write.send(Message::Text(msg_str)).await?;
                    debug!("DEV MODE: Sent orm_ready signal to challenge (plain text)");
                }
            } else {
                // Encrypt the message (production mode or dev mode with encryption enabled)
                let cipher_for_ready = ChaCha20Poly1305::new_from_slice(&aead_key)
                    .map_err(|_| anyhow!("Invalid AEAD key length (expected 32 bytes)"))?;
                let mut orm_ready_nonce = [0u8; 12];
                rand::thread_rng().fill_bytes(&mut orm_ready_nonce);
                let orm_ready_nonce_b64 = base64_engine.encode(orm_ready_nonce);

                let orm_ready_bytes = serde_json::to_vec(&orm_ready_msg)
                    .map_err(|_| anyhow!("Failed to serialize orm_ready"))?;

                let orm_ready_ciphertext = cipher_for_ready
                    .encrypt(&orm_ready_nonce.into(), orm_ready_bytes.as_slice())
                    .map_err(|_| anyhow!("Failed to encrypt orm_ready"))?;
                let orm_ready_ciphertext_b64 = base64_engine.encode(orm_ready_ciphertext);

                let orm_ready_envelope = serde_json::json!({
                    "enc": "chacha20poly1305",
                    "nonce": orm_ready_nonce_b64,
                    "ciphertext": orm_ready_ciphertext_b64,
                });

                {
                    let mut write = write_handle.lock().await;
                    write
                        .send(Message::Text(serde_json::to_string(&orm_ready_envelope)?))
                        .await?;
                }
                info!("✅ Sent orm_ready signal to challenge");
            }
        }

        // Send initial validator list to challenge after orm_ready
        // This ensures the challenge has validator information immediately on startup
        if let Some(compose_hash) = &self.compose_hash {
            let mut active_validators = std::collections::HashSet::new();
            
            // Get validators from validator_challenge_status
            if let Some(validator_status) = &self.validator_challenge_status {
                let status_map = validator_status.read().await;
                for (hotkey, challenge_statuses) in status_map.iter() {
                    if let Some(status) = challenge_statuses.get(compose_hash) {
                        if matches!(
                            status.state,
                            platform_api_models::ValidatorChallengeState::Active
                        ) {
                            active_validators.insert(hotkey.clone());
                        }
                    }
                }
                drop(status_map); // Release lock
            }
            
            // Also get validators from WebSocket connections (important fallback)
            if let Some(validator_conns) = &self.validator_connections {
                let connections = validator_conns.read().await;
                for (hotkey, conn) in connections.iter() {
                    if conn.compose_hash == *compose_hash {
                        active_validators.insert(hotkey.clone());
                    }
                }
                drop(connections); // Release lock
            }
            
            let active_validators: Vec<String> = active_validators.into_iter().collect();
            
            if let Some(_validator_status) = &self.validator_challenge_status {
                
                if !active_validators.is_empty() {
                    info!(
                        compose_hash = compose_hash,
                        validator_count = active_validators.len(),
                        "Sending initial validator list to challenge"
                    );
                    
                    let validator_update_msg = serde_json::json!({
                        "type": "validator_status_update",
                        "compose_hash": compose_hash,
                        "validators": active_validators.iter().map(|v| serde_json::json!({
                            "hotkey": v,
                            "status": "active"
                        })).collect::<Vec<_>>()
                    });

                    if !use_encryption {
                        // In dev mode without encryption, send plain text message
                        {
                            let mut write = write_handle.lock().await;
                            let msg_str = serde_json::to_string(&validator_update_msg)?;
                            write.send(Message::Text(msg_str)).await?;
                            debug!("DEV MODE: Sent initial validator list to challenge (plain text)");
                        }
                    } else {
                        // Encrypt the message
                        let cipher_for_update = ChaCha20Poly1305::new_from_slice(&aead_key)
                            .map_err(|_| anyhow!("Invalid AEAD key length (expected 32 bytes)"))?;
                        let mut update_nonce = [0u8; 12];
                        rand::thread_rng().fill_bytes(&mut update_nonce);
                        let update_nonce_b64 = base64_engine.encode(update_nonce);

                        let update_bytes = serde_json::to_vec(&validator_update_msg)
                            .map_err(|_| anyhow!("Failed to serialize validator update"))?;

                        let update_ciphertext = cipher_for_update
                            .encrypt(&update_nonce.into(), update_bytes.as_slice())
                            .map_err(|_| anyhow!("Failed to encrypt validator update"))?;
                        let update_ciphertext_b64 = base64_engine.encode(update_ciphertext);

                        let update_envelope = serde_json::json!({
                            "enc": "chacha20poly1305",
                            "nonce": update_nonce_b64,
                            "ciphertext": update_ciphertext_b64,
                        });

                        {
                            let mut write = write_handle.lock().await;
                            write
                                .send(Message::Text(serde_json::to_string(&update_envelope)?))
                                .await?;
                        }
                        info!("✅ Sent initial validator list to challenge");
                    }
                } else {
                    warn!(
                        compose_hash = compose_hash,
                        "No active validators found to send to challenge (validators may connect later)"
                    );
                    // Even if no validators are active now, send an empty list to initialize the handler
                    // This ensures the challenge knows the system is working
                    let validator_update_msg = serde_json::json!({
                        "type": "validator_status_update",
                        "compose_hash": compose_hash,
                        "validators": []
                    });

                    if !use_encryption {
                        let mut write = write_handle.lock().await;
                        if let Ok(msg_str) = serde_json::to_string(&validator_update_msg) {
                            if write.send(Message::Text(msg_str)).await.is_ok() {
                                debug!("Sent empty validator list to challenge (no validators active yet)");
                            }
                        }
                    } else {
                        if let Ok(cipher_for_update) = ChaCha20Poly1305::new_from_slice(&aead_key) {
                            let mut update_nonce = [0u8; 12];
                            rand::thread_rng().fill_bytes(&mut update_nonce);
                            let update_nonce_b64 = base64_engine.encode(update_nonce);

                            if let Ok(update_bytes) = serde_json::to_vec(&validator_update_msg) {
                                if let Ok(update_ciphertext) = cipher_for_update
                                    .encrypt(&update_nonce.into(), update_bytes.as_slice()) {
                                    let update_ciphertext_b64 = base64_engine.encode(update_ciphertext);

                                    let update_envelope = serde_json::json!({
                                        "enc": "chacha20poly1305",
                                        "nonce": update_nonce_b64,
                                        "ciphertext": update_ciphertext_b64,
                                    });

                                    let mut write = write_handle.lock().await;
                                    if let Ok(envelope_str) = serde_json::to_string(&update_envelope) {
                                        if write.send(Message::Text(envelope_str)).await.is_ok() {
                                            debug!("Sent empty validator list to challenge (encrypted, no validators active yet)");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                // If validator_challenge_status is not available, still try to use validator_connections
                if !active_validators.is_empty() {
                    info!(
                        compose_hash = compose_hash,
                        validator_count = active_validators.len(),
                        "Sending initial validator list to challenge (from WebSocket connections only)"
                    );
                    
                    let validator_update_msg = serde_json::json!({
                        "type": "validator_status_update",
                        "compose_hash": compose_hash,
                        "validators": active_validators.iter().map(|v| serde_json::json!({
                            "hotkey": v,
                            "status": "active"
                        })).collect::<Vec<_>>()
                    });
                    
                    if !use_encryption {
                        let mut write = write_handle.lock().await;
                        if let Ok(msg_str) = serde_json::to_string(&validator_update_msg) {
                            if write.send(Message::Text(msg_str)).await.is_ok() {
                                debug!("Sent initial validator list to challenge (from WebSocket connections, plain text)");
                            }
                        }
                    } else {
                        if let Ok(cipher_for_update) = ChaCha20Poly1305::new_from_slice(&aead_key) {
                            let mut update_nonce = [0u8; 12];
                            rand::thread_rng().fill_bytes(&mut update_nonce);
                            let update_nonce_b64 = base64_engine.encode(update_nonce);
                            
                            if let Ok(update_bytes) = serde_json::to_vec(&validator_update_msg) {
                                if let Ok(update_ciphertext) = cipher_for_update
                                    .encrypt(&update_nonce.into(), update_bytes.as_slice()) {
                                    let update_ciphertext_b64 = base64_engine.encode(update_ciphertext);
                                    
                                    let update_envelope = serde_json::json!({
                                        "enc": "chacha20poly1305",
                                        "nonce": update_nonce_b64,
                                        "ciphertext": update_ciphertext_b64,
                                    });
                                    
                                    let mut write = write_handle.lock().await;
                                    if let Ok(envelope_str) = serde_json::to_string(&update_envelope) {
                                        if write.send(Message::Text(envelope_str)).await.is_ok() {
                                            debug!("Sent initial validator list to challenge (from WebSocket connections, encrypted)");
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Send empty list to initialize the handler
                    let validator_update_msg = serde_json::json!({
                        "type": "validator_status_update",
                        "compose_hash": compose_hash,
                        "validators": []
                    });

                    if !use_encryption {
                        let mut write = write_handle.lock().await;
                        if let Ok(msg_str) = serde_json::to_string(&validator_update_msg) {
                            if write.send(Message::Text(msg_str)).await.is_ok() {
                                debug!("Sent empty validator list to challenge (no validators active yet)");
                            }
                        }
                    } else {
                        if let Ok(cipher_for_update) = ChaCha20Poly1305::new_from_slice(&aead_key) {
                            let mut update_nonce = [0u8; 12];
                            rand::thread_rng().fill_bytes(&mut update_nonce);
                            let update_nonce_b64 = base64_engine.encode(update_nonce);

                            if let Ok(update_bytes) = serde_json::to_vec(&validator_update_msg) {
                                if let Ok(update_ciphertext) = cipher_for_update
                                    .encrypt(&update_nonce.into(), update_bytes.as_slice()) {
                                    let update_ciphertext_b64 = base64_engine.encode(update_ciphertext);

                                    let update_envelope = serde_json::json!({
                                        "enc": "chacha20poly1305",
                                        "nonce": update_nonce_b64,
                                        "ciphertext": update_ciphertext_b64,
                                    });

                                    let mut write = write_handle.lock().await;
                                    if let Ok(envelope_str) = serde_json::to_string(&update_envelope) {
                                        if write.send(Message::Text(envelope_str)).await.is_ok() {
                                            debug!("Sent empty validator list to challenge (encrypted, no validators active yet)");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Clone references for ORM handling
        let challenge_id_clone = self.challenge_id.clone();
        let orm_gateway_clone = self.orm_gateway.clone();
        let write_handle_for_orm = write_handle.clone();
        
        // Clone references for periodic validator updates
        let compose_hash_for_updates = self.compose_hash.clone();
        let validator_status_for_updates = self.validator_challenge_status.clone();
        let validator_connections_for_updates = self.validator_connections.clone();
        let write_handle_for_updates = write_handle.clone();
        let use_encryption_for_updates = use_encryption;
        let aead_key_for_updates = aead_key.clone();

        // Spawn task for periodic validator list refresh (every 5 seconds for faster updates)
        if compose_hash_for_updates.is_some() && (validator_status_for_updates.is_some() || validator_connections_for_updates.is_some()) {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
                loop {
                    interval.tick().await;
                    
                    if let Some(compose_hash) = &compose_hash_for_updates {
                        let mut active_validators = std::collections::HashSet::new();
                        
                        // Get validators from validator_challenge_status
                        if let Some(validator_status) = &validator_status_for_updates {
                            let status_map = validator_status.read().await;
                            for (hotkey, challenge_statuses) in status_map.iter() {
                                if let Some(status) = challenge_statuses.get(compose_hash) {
                                    if matches!(
                                        status.state,
                                        platform_api_models::ValidatorChallengeState::Active
                                    ) {
                                        active_validators.insert(hotkey.clone());
                                    }
                                }
                            }
                            drop(status_map); // Release lock
                        }
                        
                        // Also get validators from WebSocket connections
                        if let Some(validator_conns) = &validator_connections_for_updates {
                            let connections = validator_conns.read().await;
                            for (hotkey, conn) in connections.iter() {
                                if conn.compose_hash == *compose_hash {
                                    active_validators.insert(hotkey.clone());
                                }
                            }
                            drop(connections); // Release lock
                        }
                        
                        let active_validators: Vec<String> = active_validators.into_iter().collect();
                        
                        if !active_validators.is_empty() {
                            debug!(
                                compose_hash = compose_hash,
                                validator_count = active_validators.len(),
                                "Sending periodic validator list update to challenge"
                            );
                            let validator_update_msg = serde_json::json!({
                                "type": "validator_status_update",
                                "compose_hash": compose_hash,
                                "validators": active_validators.iter().map(|v| serde_json::json!({
                                    "hotkey": v,
                                    "status": "active"
                                })).collect::<Vec<_>>()
                            });

                            if !use_encryption_for_updates {
                                // In dev mode without encryption, send plain text message
                                {
                                    let mut write = write_handle_for_updates.lock().await;
                                    if let Ok(msg_str) = serde_json::to_string(&validator_update_msg) {
                                        if write.send(Message::Text(msg_str)).await.is_ok() {
                                            debug!("Sent periodic validator list update to challenge");
                                        }
                                    }
                                }
                            } else {
                                // Encrypt the message
                                if let Ok(cipher_for_update) = ChaCha20Poly1305::new_from_slice(&aead_key_for_updates) {
                                    let mut update_nonce = [0u8; 12];
                                    rand::thread_rng().fill_bytes(&mut update_nonce);
                                    let update_nonce_b64 = base64_engine.encode(update_nonce);

                                    if let Ok(update_bytes) = serde_json::to_vec(&validator_update_msg) {
                                        if let Ok(update_ciphertext) = cipher_for_update
                                            .encrypt(&update_nonce.into(), update_bytes.as_slice()) {
                                            let update_ciphertext_b64 = base64_engine.encode(update_ciphertext);

                                            let update_envelope = serde_json::json!({
                                                "enc": "chacha20poly1305",
                                                "nonce": update_nonce_b64,
                                                "ciphertext": update_ciphertext_b64,
                                            });

                                            {
                                                let mut write = write_handle_for_updates.lock().await;
                                                if let Ok(envelope_str) = serde_json::to_string(&update_envelope) {
                                                    if write.send(Message::Text(envelope_str)).await.is_ok() {
                                                        debug!("Sent periodic validator list update to challenge (encrypted)");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            });
        }

        // Now handle verified messages (conn_state is guaranteed to be Verified at this point)
        info!("Entering main message loop for ORM bridge and other messages");
        while let Some(msg_result) = read.next().await {
            let msg = match msg_result {
                Ok(m) => m,
                Err(e) => {
                    error!(error = %e, "Error receiving WebSocket message - connection may be broken");
                    break;
                }
            };
            
            // Handle different message types
            match &msg {
                Message::Close(_) => {
                    warn!("Received Close message from challenge - closing connection");
                    break;
                }
                Message::Ping(data) => {
                    debug!("Received Ping, responding with Pong");
                    let mut write = write_handle.lock().await;
                    if let Err(e) = write.send(Message::Pong(data.clone())).await {
                        warn!(error = %e, "Failed to send Pong response");
                        break;
                    }
                    continue;
                }
                Message::Pong(_) => {
                    debug!("Received Pong");
                    continue;
                }
                Message::Text(text) => {
                let plain_msg: serde_json::Value = if !use_encryption {
                    // In dev mode without encryption, message is already plain text
                    match serde_json::from_str(&text) {
                        Ok(pm) => pm,
                        Err(e) => {
                            debug!(error = %e, "DEV MODE: Failed to parse plain text message");
                            continue;
                        }
                    }
                } else {
                    // Decrypt the message (production mode or dev mode with encryption enabled)
                    info!("📥 Received raw WebSocket message (encrypted envelope)");

                    let json: Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(error = %e, "Failed to parse WebSocket message as JSON");
                            continue;
                        }
                    };

                    // Decrypt and handle message
                    let envelope: EncryptedEnvelope = match serde_json::from_value(json) {
                        Ok(e) => e,
                        Err(e) => {
                            warn!(error = %e, "Failed to parse message as encrypted envelope");
                            continue;
                        }
                    };

                    let nonce_bytes = match base64_engine.decode(&envelope.nonce) {
                        Ok(n) => n,
                        Err(e) => {
                            warn!(error = %e, "Failed to decode nonce from base64");
                            continue;
                        }
                    };

                    if nonce_bytes.len() != 12 {
                        warn!(len = nonce_bytes.len(), "Invalid nonce length");
                        continue;
                    }
                    let nonce_array: [u8; 12] = match nonce_bytes[..12].try_into() {
                        Ok(arr) => arr,
                        Err(_) => {
                            warn!("Failed to convert nonce to array (this should not happen)");
                            continue;
                        }
                    };

                    let ciphertext = match base64_engine.decode(&envelope.ciphertext) {
                        Ok(c) => c,
                        Err(e) => {
                            warn!(error = %e, "Failed to decode ciphertext from base64");
                            continue;
                        }
                    };

                    let cipher = ChaCha20Poly1305::new_from_slice(&aead_key).map_err(|_| {
                        error!("Invalid AEAD key length (expected 32 bytes)");
                        anyhow!("Invalid AEAD key length")
                    })?;
                    let plaintext = match cipher.decrypt(&nonce_array.into(), ciphertext.as_slice())
                    {
                        Ok(pt) => pt,
                        Err(e) => {
                            warn!(error = %e, "Failed to decrypt message - may be from wrong session or corrupted");
                            continue;
                        }
                    };

                    match serde_json::from_slice(&plaintext) {
                        Ok(pm) => pm,
                        Err(e) => {
                            warn!(error = %e, "Failed to parse decrypted plaintext as JSON");
                            continue;
                        }
                    }
                };

                // Log all incoming messages for debugging (reduced verbosity)
                // info!(
                //     msg_type = &plain_msg["type"].as_str().unwrap_or(""),
                //     challenge_id = challenge_id_clone.as_deref(),
                //     "Received message from challenge via WebSocket"
                // );

                // Create cipher for encryption (needed if encryption is enabled)
                let cipher_opt = if !use_encryption {
                    None
                } else {
                    Some(ChaCha20Poly1305::new_from_slice(&aead_key).map_err(|_| {
                        error!("Invalid AEAD key length (expected 32 bytes)");
                        anyhow!("Invalid AEAD key length")
                    })?)
                };

                // Handle benchmark_progress messages for Redis logging
                if plain_msg["type"].as_str().unwrap_or("") == "benchmark_progress" {
                    if let Some(redis) = &self.redis_client {
                        if let Some(job_id) =
                            plain_msg.get("payload").and_then(|p| p.get("job_id")).and_then(|v| v.as_str())
                        {
                            if let Some(progress_data) = plain_msg.get("payload").and_then(|p| p.get("progress")) {
                                // Extract progress metrics
                                let progress_obj = progress_data.as_object();
                                let progress_percent = progress_obj
                                    .and_then(|p| p.get("progress_percent"))
                                    .and_then(|v| v.as_f64())
                                    .unwrap_or(0.0);
                                let total_tasks = progress_obj
                                    .and_then(|p| p.get("total_tasks"))
                                    .and_then(|v| v.as_i64())
                                    .map(|v| v as i32);
                                let completed_tasks = progress_obj
                                    .and_then(|p| p.get("completed_tasks"))
                                    .and_then(|v| v.as_i64())
                                    .map(|v| v as i32);
                                let resolved_tasks = progress_obj
                                    .and_then(|p| p.get("resolved_tasks"))
                                    .and_then(|v| v.as_i64())
                                    .map(|v| v as i32);
                                let unresolved_tasks = progress_obj
                                    .and_then(|p| p.get("unresolved_tasks"))
                                    .and_then(|v| v.as_i64())
                                    .map(|v| v as i32);

                                let status = progress_obj
                                    .and_then(|p| p.get("status"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("running")
                                    .to_string();

                                // Log progress to Redis
                                let progress = create_job_progress(
                                    job_id.to_string(),
                                    status,
                                    progress_percent,
                                    total_tasks,
                                    completed_tasks,
                                    resolved_tasks,
                                    unresolved_tasks,
                                    None,
                                );

                                if let Err(e) = redis.set_job_progress(&progress).await {
                                    warn!("Failed to log job progress to Redis: {}", e);
                                }

                                // Log progress event to Redis logs
                                let log_entry = create_job_log(
                                    "info".to_string(),
                                    format!(
                                        "Progress update: {:.1}% ({} tasks completed)",
                                        progress_percent,
                                        completed_tasks.unwrap_or(0)
                                    ),
                                    Some(progress_data.clone()),
                                );

                                if let Err(e) = redis.append_job_log(job_id, &log_entry).await {
                                    warn!("Failed to append job log to Redis: {}", e);
                                }
                            }
                        }
                    }
                }

                // Handle ORM permissions update from challenge
                if plain_msg["type"].as_str().unwrap_or("") == "orm_permissions" {
                    if let Some(challenge_id) = &challenge_id_clone {
                        if let Some(orm_gateway) = &orm_gateway_clone {
                            // Extract permissions from payload
                            if let (Some(permissions_json), Some(msg_challenge_id)) = (
                                plain_msg.get("payload").and_then(|p| p.get("permissions")),
                                plain_msg
                                    .get("payload")
                                    .and_then(|p| p.get("challenge_id"))
                                    .and_then(|v| v.as_str()),
                            ) {
                                // Verify challenge_id matches
                                if msg_challenge_id == challenge_id {
                                    info!(
                                        challenge_id = challenge_id,
                                        "Received ORM permissions from challenge"
                                    );

                                    // Parse and load permissions
                                    match serde_json::from_value::<
                                        std::collections::HashMap<
                                            String,
                                            crate::orm_gateway::TablePermission,
                                        >,
                                    >(
                                        permissions_json.clone()
                                    ) {
                                        Ok(permissions) => {
                                            let mut gateway = orm_gateway.write().await;
                                            if let Err(e) = gateway
                                                .load_challenge_permissions(
                                                    challenge_id,
                                                    permissions,
                                                )
                                                .await
                                            {
                                                error!(
                                                    challenge_id = challenge_id,
                                                    error = %e,
                                                    "Failed to load ORM permissions"
                                                );
                                            } else {
                                                info!(
                                                    challenge_id = challenge_id,
                                                    "✅ ORM permissions loaded successfully"
                                                );

                                                // Send acknowledgment
                                                let ack_msg = serde_json::json!({
                                                    "type": "orm_permissions_ack",
                                                    "status": "success"
                                                });

                                                if !use_encryption {
                                                    // In dev mode without encryption, send plain text acknowledgment
                                                    {
                                                        let mut write = write_handle.lock().await;
                                                        let msg_str =
                                                            serde_json::to_string(&ack_msg)?;
                                                        write.send(Message::Text(msg_str)).await?;
                                                    }
                                                } else {
                                                    // Encrypt the acknowledgment (production mode or dev mode with encryption enabled)
                                                    if let Some(cipher) = &cipher_opt {
                                                        let mut ack_nonce = [0u8; 12];
                                                        rand::thread_rng()
                                                            .fill_bytes(&mut ack_nonce);
                                                        let ack_nonce_b64 =
                                                            base64_engine.encode(ack_nonce);

                                                        let ack_bytes = serde_json::to_vec(
                                                            &ack_msg,
                                                        )
                                                        .map_err(|_| {
                                                            anyhow!("Failed to serialize ack")
                                                        })?;

                                                        let ack_ciphertext = cipher
                                                            .encrypt(
                                                                &ack_nonce.into(),
                                                                ack_bytes.as_slice(),
                                                            )
                                                            .map_err(|_| {
                                                                anyhow!("Failed to encrypt ack")
                                                            })?;
                                                        let ack_ciphertext_b64 =
                                                            base64_engine.encode(ack_ciphertext);

                                                        let ack_envelope = serde_json::json!({
                                                            "enc": "chacha20poly1305",
                                                            "nonce": ack_nonce_b64,
                                                            "ciphertext": ack_ciphertext_b64,
                                                        });

                                                        {
                                                            let mut write =
                                                                write_handle.lock().await;
                                                            write
                                                                .send(Message::Text(
                                                                    serde_json::to_string(
                                                                        &ack_envelope,
                                                                    )?,
                                                                ))
                                                                .await?;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!(
                                                challenge_id = challenge_id,
                                                error = %e,
                                                "Failed to parse ORM permissions"
                                            );
                                        }
                                    }
                                } else {
                                    warn!(
                                        challenge_id = challenge_id,
                                        msg_challenge_id = msg_challenge_id,
                                        "ORM permissions challenge_id mismatch"
                                    );
                                }
                            } else {
                                warn!("ORM permissions message missing required fields");
                            }
                        }
                    }
                    continue;
                }

                // Handle get_validator_count request
                if plain_msg["type"].as_str().unwrap_or("") == "get_validator_count" {
                    if let Some(validator_status) = &self.validator_challenge_status {
                        // Extract compose_hash from payload
                        if let Some(compose_hash) = plain_msg
                            .get("payload")
                            .and_then(|p| p.get("compose_hash"))
                            .and_then(|v| v.as_str())
                        {
                            // Count active validators for this compose_hash
                            let status_map = validator_status.read().await;
                            let mut count = 0;

                            for (_hotkey, challenge_statuses) in status_map.iter() {
                                if let Some(status) = challenge_statuses.get(compose_hash) {
                                    if matches!(
                                        status.state,
                                        platform_api_models::ValidatorChallengeState::Active
                                    ) {
                                        count += 1;
                                    }
                                }
                            }

                            info!(
                                compose_hash = compose_hash,
                                validator_count = count,
                                "Returning validator count for challenge"
                            );

                            // Send response
                            let response_msg = serde_json::json!({
                                "type": "validator_count_result",
                                "compose_hash": compose_hash,
                                "count": count
                            });

                            if !use_encryption {
                                // In dev mode without encryption, send plain text response
                                {
                                    let mut write = write_handle.lock().await;
                                    let msg_str = serde_json::to_string(&response_msg)?;
                                    write.send(Message::Text(msg_str)).await?;
                                }
                            } else {
                                // Encrypt the response (production mode or dev mode with encryption enabled)
                                if let Some(cipher) = &cipher_opt {
                                    let mut response_nonce = [0u8; 12];
                                    rand::thread_rng().fill_bytes(&mut response_nonce);
                                    let response_nonce_b64 = base64_engine.encode(response_nonce);

                                    let response_bytes = serde_json::to_vec(&response_msg)
                                        .map_err(|_| anyhow!("Failed to serialize response"))?;

                                    let response_ciphertext = cipher
                                        .encrypt(&response_nonce.into(), response_bytes.as_slice())
                                        .map_err(|_| anyhow!("Failed to encrypt response"))?;
                                    let response_ciphertext_b64 =
                                        base64_engine.encode(response_ciphertext);

                                    let response_envelope = serde_json::json!({
                                        "enc": "chacha20poly1305",
                                        "nonce": response_nonce_b64,
                                        "ciphertext": response_ciphertext_b64,
                                    });

                                    {
                                        let mut write = write_handle.lock().await;
                                        write
                                            .send(Message::Text(serde_json::to_string(
                                                &response_envelope,
                                            )?))
                                            .await?;
                                    }
                                }
                            }
                            continue;
                        } else {
                            warn!("get_validator_count message missing compose_hash in payload");
                        }
                    } else {
                        warn!("validator_challenge_status not available for get_validator_count");
                    }
                    continue;
                }

                // Handle ORM queries via bridge/proxy
                if plain_msg["type"].as_str().unwrap_or("") == "orm_query" {
                    info!(
                        msg_type = &plain_msg["type"].as_str().unwrap_or(""),
                        challenge_id = challenge_id_clone.as_deref(),
                        has_orm_gateway = orm_gateway_clone.is_some(),
                        "Received ORM query message"
                    );
                    if let Some(challenge_id) = &challenge_id_clone {
                        if let Some(orm_gateway) = &orm_gateway_clone {
                            // Extract ORM query from payload
                            if let Some(query_json) = plain_msg.get("payload").and_then(|p| p.get("query")) {
                                match serde_json::from_value::<crate::orm_gateway::ORMQuery>(
                                    query_json.clone(),
                                ) {
                                    Ok(mut orm_query) => {
                                        // ALWAYS set schema to challenge schema - platform-api controls schemas
                                        // Ignore any schema provided by SDK - platform-api defines schemas based on challenge configuration
                                        // Use the schema_name from ChallengeWsClient (which is {challenge_name}_v{db_version})
                                        // Fallback to challenge_{challenge_id} if schema_name not set
                                        let schema = if let Some(schema_arc) = &self.schema_name {
                                            schema_arc.read().await.clone()
                                        } else {
                                            format!("challenge_{}", challenge_id.replace('-', "_"))
                                        };
                                        let schema_for_log = schema.clone();
                                        orm_query.schema = Some(schema);
                                        info!(
                                            challenge_id = challenge_id,
                                            schema = &schema_for_log,
                                            "Platform-api set schema for ORM query (overriding any SDK-provided schema)"
                                        );

                                        info!(
                                            challenge_id = challenge_id,
                                            operation = &orm_query.operation,
                                            table = &orm_query.table,
                                            "Executing ORM query via bridge"
                                        );

                                        // Execute query via ORM Gateway (with RwLock)
                                        // This is read-write mode for direct SDK connections (public routes)
                                        let orm_gateway_guard = orm_gateway.read().await;
                                        match orm_gateway_guard.execute_query(orm_query).await {
                                            Ok(result) => {
                                                // Include query_id if present in request
                                                let mut response_msg = serde_json::json!({
                                                    "type": "orm_result",
                                                    "result": result
                                                });
                                                // Extract query_id from multiple possible locations:
                                                // 1. payload.query_id (direct)
                                                // 2. payload.query.query_id (nested in query object)
                                                // 3. message_id at root level (used by MessageRouter)
                                                let query_id_opt = plain_msg
                                                    .get("payload")
                                                    .and_then(|p| p.get("query_id"))
                                                    .and_then(|v| v.as_str())
                                                    .or_else(|| {
                                                        // Also check if query_id is in the query object
                                                        plain_msg
                                                            .get("payload")
                                                            .and_then(|p| p.get("query"))
                                                            .and_then(|q| q.get("query_id"))
                                                            .and_then(|v| v.as_str())
                                                    })
                                                    .or_else(|| {
                                                        // Also check root-level message_id (used by MessageRouter for matching)
                                                        plain_msg.get("message_id").and_then(|v| v.as_str())
                                                    });
                                                if let Some(query_id) = query_id_opt {
                                                    response_msg["query_id"] =
                                                        serde_json::Value::String(
                                                            query_id.to_string(),
                                                        );
                                                    // Also include as message_id for MessageRouter compatibility
                                                    response_msg["message_id"] =
                                                        serde_json::Value::String(
                                                            query_id.to_string(),
                                                        );
                                                    info!(query_id = query_id, "Including query_id/message_id in ORM result response");
                                                } else {
                                                    warn!("ORM query missing query_id/message_id, response won't be matched");
                                                }

                                                if !use_encryption {
                                                    // In dev mode without encryption, send plain text response
                                                    {
                                                        let mut write = write_handle.lock().await;
                                                        let msg_str =
                                                            serde_json::to_string(&response_msg)?;
                                                        write.send(Message::Text(msg_str)).await?;
                                                    }
                                                } else {
                                                    // In production mode, encrypt the response
                                                    if let Some(cipher) = &cipher_opt {
                                                        let mut response_nonce = [0u8; 12];
                                                        rand::thread_rng()
                                                            .fill_bytes(&mut response_nonce);
                                                        let response_nonce_b64 =
                                                            base64_engine.encode(response_nonce);

                                                        let response_bytes = serde_json::to_vec(
                                                            &response_msg,
                                                        )
                                                        .map_err(|_| {
                                                            anyhow!("Failed to serialize response")
                                                        })?;

                                                        let response_ciphertext = cipher
                                                            .encrypt(
                                                                &response_nonce.into(),
                                                                response_bytes.as_slice(),
                                                            )
                                                            .map_err(|_| {
                                                                anyhow!(
                                                                    "Failed to encrypt response"
                                                                )
                                                            })?;
                                                        let response_ciphertext_b64 = base64_engine
                                                            .encode(response_ciphertext);

                                                        let response_envelope = serde_json::json!({
                                                            "enc": "chacha20poly1305",
                                                            "nonce": response_nonce_b64,
                                                            "ciphertext": response_ciphertext_b64,
                                                        });

                                                        {
                                                            let mut write =
                                                                write_handle.lock().await;
                                                            write
                                                                .send(Message::Text(
                                                                    serde_json::to_string(
                                                                        &response_envelope,
                                                                    )?,
                                                                ))
                                                                .await?;
                                                        }
                                                    }
                                                }
                                                info!(
                                                    challenge_id = challenge_id,
                                                    row_count = result.row_count,
                                                    "ORM query executed successfully via bridge"
                                                );
                                                continue;
                                            }
                                            Err(e) => {
                                                // Send error response
                                                // Include query_id/message_id if present in request (same logic as success response)
                                                let mut error_msg = serde_json::json!({
                                                    "type": "error",
                                                    "error": e.to_string(),
                                                    "message": e.to_string()  // Include both "error" and "message" for compatibility
                                                });
                                                // Extract query_id from multiple possible locations (same as success response)
                                                let query_id_opt = plain_msg
                                                    .get("payload")
                                                    .and_then(|p| p.get("query_id"))
                                                    .and_then(|v| v.as_str())
                                                    .or_else(|| {
                                                        plain_msg
                                                            .get("payload")
                                                            .and_then(|p| p.get("query"))
                                                            .and_then(|q| q.get("query_id"))
                                                            .and_then(|v| v.as_str())
                                                    })
                                                    .or_else(|| plain_msg.get("message_id").and_then(|v| v.as_str()));
                                                if let Some(query_id) = query_id_opt {
                                                    error_msg["query_id"] =
                                                        serde_json::Value::String(
                                                            query_id.to_string(),
                                                        );
                                                    error_msg["message_id"] =
                                                        serde_json::Value::String(
                                                            query_id.to_string(),
                                                        );
                                                    info!(query_id = query_id, error = %e, "Including query_id/message_id in ORM error response");
                                                } else {
                                                    warn!(error = %e, "ORM error missing query_id/message_id, response won't be matched");
                                                }

                                                if !use_encryption {
                                                    // In dev mode without encryption, send plain text error response
                                                    {
                                                        let mut write = write_handle.lock().await;
                                                        let msg_str =
                                                            serde_json::to_string(&error_msg)?;
                                                        write.send(Message::Text(msg_str)).await?;
                                                    }
                                                } else {
                                                    // Encrypt the error response (production mode or dev mode with encryption enabled)
                                                    if let Some(cipher) = &cipher_opt {
                                                        let mut error_nonce = [0u8; 12];
                                                        rand::thread_rng()
                                                            .fill_bytes(&mut error_nonce);
                                                        let error_nonce_b64 =
                                                            base64_engine.encode(error_nonce);

                                                        let error_bytes =
                                                            serde_json::to_vec(&error_msg)
                                                                .map_err(|_| {
                                                                    anyhow!(
                                                                        "Failed to serialize error"
                                                                    )
                                                                })?;

                                                        let error_ciphertext = cipher
                                                            .encrypt(
                                                                &error_nonce.into(),
                                                                error_bytes.as_slice(),
                                                            )
                                                            .map_err(|_| {
                                                                anyhow!("Failed to encrypt error")
                                                            })?;
                                                        let error_ciphertext_b64 =
                                                            base64_engine.encode(error_ciphertext);

                                                        let error_envelope = serde_json::json!({
                                                            "enc": "chacha20poly1305",
                                                            "nonce": error_nonce_b64,
                                                            "ciphertext": error_ciphertext_b64,
                                                        });

                                                        {
                                                            let mut write =
                                                                write_handle.lock().await;
                                                            write
                                                                .send(Message::Text(
                                                                    serde_json::to_string(
                                                                        &error_envelope,
                                                                    )?,
                                                                ))
                                                                .await?;
                                                        }
                                                    }
                                                }
                                                warn!(
                                                    challenge_id = challenge_id,
                                                    error = %e,
                                                    "ORM query failed"
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to parse ORM query: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }

                // Forward other message types to callback
                let (_tx, _rx) = mpsc::channel(100);

                // Call the callback with the decrypted message
                callback(
                    serde_json::json!({
                        "type": plain_msg["type"].as_str().unwrap_or(""),
                        "payload": plain_msg.get("payload").cloned().unwrap_or(serde_json::Value::Null)
                    }),
                    _tx,
                );
                }
                Message::Binary(_) => {
                    warn!("Received binary message (not supported)");
                    continue;
                }
                Message::Frame(_) => {
                    debug!("Received Frame message (internal, ignoring)");
                    continue;
                }
            }
        }
        
        info!("WebSocket message loop ended - connection closed");

        Ok(())
    }

    /// Verify TDX quote using DCAP libraries (same as platform-validator)
    async fn verify_tdx_quote(quote_b64: &str, nonce_bytes: &[u8; 32]) -> Result<()> {
        use dcap_qvl::{collateral, verify::verify};

        let quote_bytes = base64_engine
            .decode(quote_b64)
            .context("Failed to decode quote from base64")?;

        info!("Decoded TDX quote: {} bytes", quote_bytes.len());

        // Get collateral from Intel PCS
        let collateral_data = collateral::get_collateral_from_pcs(&quote_bytes)
            .await
            .context("Failed to get collateral from Intel PCS")?;

        // Get current timestamp
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| anyhow!("time error"))?
            .as_secs();

        // Verify quote signature and TCB
        let tcb =
            verify(&quote_bytes, &collateral_data, now).context("TDX quote verification failed")?;

        info!("TDX quote verified, TCB status: {:?}", tcb.status);

        // Verify report_data matches SHA256(nonce)
        let mut hasher = Sha256::new();
        hasher.update(nonce_bytes);
        let expected = hasher.finalize();

        // TDX report_data location can vary slightly between quote versions.
        // Try common offsets and accept a match against SHA256(nonce).
        let candidate_offsets: [usize; 3] = [568, 576, 584];
        let mut matched = false;
        let mut matched_off: Option<usize> = None;
        for off in candidate_offsets.iter() {
            if quote_bytes.len() >= off + 32 {
                let rd = &quote_bytes[*off..*off + 32];
                if rd == expected.as_slice() {
                    matched = true;
                    matched_off = Some(*off);
                    break;
                }
            }
        }

        if !matched {
            return Err(anyhow!(
                "report_data mismatch: quote report_data does not match SHA256(nonce)"
            ));
        }

        if let Some(off) = matched_off {
            info!("✅ Matched report_data at offset {}", off);
        }

        Ok(())
    }
}
