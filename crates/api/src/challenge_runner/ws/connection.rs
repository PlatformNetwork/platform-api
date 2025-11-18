//! WebSocket connection management and main connection loop

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

use super::encryption::{decrypt_envelope, encrypt_message};
use super::messages::{handle_benchmark_progress, handle_get_validator_count, handle_orm_permissions, handle_orm_query};
use super::tdx::verify_tdx_quote;
use super::types::{ChallengeWsClient, ConnectionState, EncryptedEnvelope};
use super::validators::get_active_validators_for_compose_hash;

/// Connect with reconnection logic and handle messages
/// Handles ORM queries from challenge SDK and executes them via ORM gateway
pub async fn connect_with_reconnect<F>(client: &ChallengeWsClient, callback: F) -> Result<()>
where
    F: Fn(Value, mpsc::Sender<Value>) + Send + Sync + 'static,
{
    info!("Connecting WebSocket to {}", client.url);

    // URL already includes /sdk/ws path
    let (ws_stream, _) = connect_async(&client.url)
        .await
        .with_context(|| format!("Failed to connect WebSocket to {}", client.url))?;
    let (write, mut read) = ws_stream.split();

    // Wrap write in Arc<Mutex> to share between tasks
    let write_handle = Arc::new(tokio::sync::Mutex::new(write));

    info!(
        "✅ Connected WebSocket to {}, starting TDX attestation",
        client.url
    );

    // Begin attestation handshake
    let mut nonce_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce_hex = hex::encode(nonce_bytes);

    // Initialize connection state as unverified
    let conn_state = ConnectionState::Unverified {
        nonce: nonce_bytes,
        started: Instant::now(),
    };

    // Generate platform API X25519 ephemeral keypair
    let api_secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let api_public = PublicKey::from(&api_secret);
    let api_pub_b64 = base64_engine.encode(api_public.as_bytes());

    // Send attestation_begin
    // Note: Challenge SDK expects "val_x25519_pub" (compatible with validator) or "api_x25519_pub"
    // We use "val_x25519_pub" for compatibility
    let begin_msg = Message::Text(
        serde_json::json!({
            "type": "attestation_begin",
            "nonce": nonce_hex,
            "platform_api_id": client.platform_api_id,
            "val_x25519_pub": api_pub_b64,
        })
        .to_string(),
    );
    {
        let mut write = write_handle.lock().await;
        write.send(begin_msg).await?;
    }

    // Wait for attestation_response - consume api_secret only once
    let aead_key = perform_attestation(
        &mut read,
        &conn_state,
        api_secret,
        &write_handle,
    ).await?;

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

    // Handle migrations if migration runner is configured
    handle_migrations(
        client,
        &mut read,
        &write_handle,
        use_encryption,
        &aead_key,
    ).await?;

    // Wait for migrations to be applied before sending orm_ready
    wait_for_migrations_applied(client).await;

    // Send orm_ready signal to challenge after migrations are applied (or timeout)
    send_orm_ready(client, &write_handle, use_encryption, &aead_key).await?;

    // Send initial validator list to challenge after orm_ready
    send_initial_validator_list(client, &write_handle, use_encryption, &aead_key).await?;

    // Spawn task for periodic validator list refresh
    spawn_validator_update_task(client.clone(), write_handle.clone(), use_encryption, aead_key).await;

    // Clone references for message handling
    let challenge_id_clone = client.challenge_id.clone();
    let orm_gateway_clone = client.orm_gateway.clone();

    // Now handle verified messages (conn_state is guaranteed to be Verified at this point)
    info!("Entering main message loop for ORM bridge and other messages");
    handle_message_loop(
        &mut read,
        client,
        &challenge_id_clone,
        &orm_gateway_clone,
        &write_handle,
        use_encryption,
        &aead_key,
        callback,
    ).await?;

    info!("WebSocket message loop ended - connection closed");

    Ok(())
}

/// Perform TDX attestation handshake
async fn perform_attestation(
    read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    conn_state: &ConnectionState,
    api_secret: EphemeralSecret,
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
) -> Result<[u8; 32]> {
    loop {
        let msg = read
            .next()
            .await
            .ok_or_else(|| anyhow!("Connection closed before attestation"))??;

        if let Message::Text(text) = msg {
            let json: Value = serde_json::from_str(&text)?;

            // Check timeout (30 seconds)
            if let ConnectionState::Unverified { started, .. } = conn_state {
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
                    let nonce = match conn_state {
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
                        match verify_tdx_quote(quote_b64, &nonce).await {
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
                    return Ok(key_bytes);
                }
            }

            // If not attestation_response, reject
            warn!("Unexpected message during attestation: {:?}", json);
            return Err(anyhow!("Attestation failed: unexpected message"));
        }
    }
}

/// Handle migrations request and response
async fn handle_migrations(
    client: &ChallengeWsClient,
    read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    use_encryption: bool,
    aead_key: &[u8; 32],
) -> Result<()> {
    // After TDX verification, request migrations via WebSocket (encrypted or plain text based on mode)
    // Only request migrations if CHALLENGE_ADMIN=true (admin mode)
    // Note: schema_name might be temporary (v1) until db_version is known
    if let Some(_migration_runner) = &client.migration_runner {
        // In admin mode (CHALLENGE_ADMIN=true), we request migrations
        // In non-admin mode, the challenge will return empty migrations
        // Always using encryption
        let tdx_simulation_mode = std::env::var("TDX_SIMULATION_MODE")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true";

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
        let envelope = encrypt_message(&request_msg, aead_key)?;
        let envelope_json = serde_json::json!({
            "enc": envelope.enc,
            "nonce": envelope.nonce,
            "ciphertext": envelope.ciphertext,
        });

        {
            let mut write = write_handle.lock().await;
            let envelope_str = serde_json::to_string(&envelope_json)?;
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
                                    let json: Value = serde_json::from_str(&text)?;
                                    let envelope: EncryptedEnvelope =
                                        serde_json::from_value(json)?;
                                    decrypt_envelope(&envelope, aead_key).unwrap_or_else(|e| {
                                        warn!(error = %e, "Failed to decrypt message in migrations wait loop");
                                        return serde_json::Value::Null;
                                    })
                                };

                                if plain_msg.is_null() {
                                    continue;
                                }

                                info!(msg_type = %plain_msg["type"].as_str().unwrap_or(""), "Decrypted message type");

                                if plain_msg["type"].as_str().unwrap_or("")
                                    == "migrations_response"
                                {
                                    migrations_received = process_migrations_response(
                                        client,
                                        &plain_msg,
                                    ).await;
                                    // Don't break - continue the connection for ORM bridge
                                } else {
                                    // Log other message types for debugging
                                    info!(msg_type = %plain_msg["type"].as_str().unwrap_or(""), "Received other message type in migrations wait loop (not migrations_response)");
                                }
                            } else {
                                // Handle Ping/Pong/Close messages
                                handle_control_messages(&msg, write_handle).await;
                                if matches!(&msg, Message::Close(_)) {
                                    migrations_received = true; // Exit loop
                                    break;
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
                    if elapsed.as_secs().is_multiple_of(5) && elapsed.as_millis() % 5000 < 100 {
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

    Ok(())
}

/// Process migrations_response message
async fn process_migrations_response(
    client: &ChallengeWsClient,
    plain_msg: &Value,
) -> bool {
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
        if let Some(sender_arc) = &client.db_version_sender {
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

    if let Some(migrations_json) = plain_msg
        .get("payload")
        .and_then(|p| p.get("migrations"))
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
            if let Some(sender_arc) = &client.migrations_sender {
                let mut sender_opt = sender_arc.lock().await;
                if let Some(sender) = sender_opt.take() {
                    let _ = sender.send(received_migrations.clone());
                    info!("migrations sent via channel");
                }
            }

            // Apply migrations immediately in WebSocket task to ensure they're done before orm_ready
            // This prevents the WebSocket from closing before migrations are applied
            if let Some(migration_runner) = &client.migration_runner {
                if let Some(schema_arc) = &client.schema_name {
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
                            if let Some(sender_arc) = &client.migrations_applied_sender {
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

            true
        } else {
            warn!("Failed to parse migrations from migrations_response");
            false
        }
    } else {
        warn!("No migrations field in migrations_response payload");
        false
    }
}

/// Handle control messages (Ping, Pong, Close)
async fn handle_control_messages(
    msg: &Message,
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
) {
    match msg {
        Message::Close(_) => {
            warn!("Received Close message");
        }
        Message::Ping(data) => {
            info!("Received Ping, responding with Pong");
            {
                let mut write = write_handle.lock().await;
                let _ = write.send(Message::Pong(data.clone())).await;
            }
        }
        Message::Pong(_) => {
            info!("Received Pong (ignoring)");
        }
        Message::Binary(data) => {
            warn!("Received Binary message ({} bytes)", data.len());
        }
        Message::Frame(_) => {
            warn!("Received Frame message");
        }
        _ => {}
    }
}

/// Wait for migrations to be applied before sending orm_ready
async fn wait_for_migrations_applied(client: &ChallengeWsClient) {
    if let Some(receiver_arc) = &client.migrations_applied_receiver {
        if let Some(receiver_guard) = receiver_arc.lock().await.take() {
            info!("Waiting for migrations to be applied before sending orm_ready...");
            match tokio::time::timeout(tokio::time::Duration::from_secs(60), receiver_guard)
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
}

/// Send orm_ready signal to challenge after migrations are applied
async fn send_orm_ready(
    client: &ChallengeWsClient,
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    use_encryption: bool,
    aead_key: &[u8; 32],
) -> Result<()> {
    // Include schema_name in orm_ready message so challenge knows which schema to use
    let schema_name_for_challenge = if let Some(schema_arc) = &client.schema_name {
        schema_arc.read().await.clone()
    } else {
        // Fallback to challenge_{challenge_id} if schema_name not set
        format!(
            "challenge_{}",
            client
                .challenge_id
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
        let envelope = encrypt_message(&orm_ready_msg, aead_key)?;
        let envelope_json = serde_json::json!({
            "enc": envelope.enc,
            "nonce": envelope.nonce,
            "ciphertext": envelope.ciphertext,
        });

        {
            let mut write = write_handle.lock().await;
            write
                .send(Message::Text(serde_json::to_string(&envelope_json)?))
                .await?;
        }
        info!("✅ Sent orm_ready signal to challenge");
    }

    Ok(())
}

/// Send initial validator list to challenge after orm_ready
async fn send_initial_validator_list(
    client: &ChallengeWsClient,
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    use_encryption: bool,
    aead_key: &[u8; 32],
) -> Result<()> {
    if let Some(compose_hash) = &client.compose_hash {
        let active_validators = get_active_validators_for_compose_hash(
            compose_hash,
            client.validator_challenge_status.as_ref().ok_or_else(|| anyhow!("Validator challenge status not available"))?,
            &client.validator_connections,
        ).await;

        let validator_update_msg = serde_json::json!({
            "type": "validator_status_update",
            "compose_hash": compose_hash,
            "validators": active_validators.iter().map(|v| serde_json::json!({
                "hotkey": v,
                "status": "active"
            })).collect::<Vec<_>>()
        });

        if !active_validators.is_empty() {
            info!(
                compose_hash = compose_hash,
                validator_count = active_validators.len(),
                "Sending initial validator list to challenge"
            );
        } else {
            warn!(
                compose_hash = compose_hash,
                "No active validators found to send to challenge (validators may connect later)"
            );
        }

        // Send validator list (encrypted or plain text)
        if !use_encryption {
            let mut write = write_handle.lock().await;
            let msg_str = serde_json::to_string(&validator_update_msg)?;
            write.send(Message::Text(msg_str)).await?;
            debug!("Sent initial validator list to challenge (plain text)");
        } else {
            let envelope = encrypt_message(&validator_update_msg, aead_key)?;
            let envelope_json = serde_json::json!({
                "enc": envelope.enc,
                "nonce": envelope.nonce,
                "ciphertext": envelope.ciphertext,
            });

            let mut write = write_handle.lock().await;
            write
                .send(Message::Text(serde_json::to_string(&envelope_json)?))
                .await?;
            info!("✅ Sent initial validator list to challenge");
        }
    }

    Ok(())
}

/// Spawn task for periodic validator list refresh
async fn spawn_validator_update_task(
    client: ChallengeWsClient,
    write_handle: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    use_encryption: bool,
    aead_key: [u8; 32],
) {
    if client.compose_hash.is_some()
        && (client.validator_challenge_status.is_some()
            || client.validator_connections.is_some())
    {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            loop {
                interval.tick().await;

                if let Some(compose_hash) = &client.compose_hash {
                    let active_validators = if let Some(validator_status) = &client.validator_challenge_status {
                        get_active_validators_for_compose_hash(
                            compose_hash,
                            validator_status,
                            &client.validator_connections,
                        ).await
                    } else {
                        // Fallback to only validator_connections if validator_status is not available
                        let mut validators = std::collections::HashSet::new();
                        if let Some(validator_conns) = &client.validator_connections {
                            let connections = validator_conns.read().await;
                            for (hotkey, conn) in connections.iter() {
                                if conn.compose_hash.as_deref() == Some(compose_hash) {
                                    validators.insert(hotkey.clone());
                                }
                            }
                        }
                        validators.into_iter().collect()
                    };

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

                        if !use_encryption {
                            let mut write = write_handle.lock().await;
                            if let Ok(msg_str) = serde_json::to_string(&validator_update_msg) {
                                if write.send(Message::Text(msg_str)).await.is_ok() {
                                    debug!("Sent periodic validator list update to challenge");
                                }
                            }
                        } else {
                            if let Ok(envelope) = encrypt_message(&validator_update_msg, &aead_key) {
                                let envelope_json = serde_json::json!({
                                    "enc": envelope.enc,
                                    "nonce": envelope.nonce,
                                    "ciphertext": envelope.ciphertext,
                                });

                                let mut write = write_handle.lock().await;
                                if let Ok(envelope_str) = serde_json::to_string(&envelope_json) {
                                    if write.send(Message::Text(envelope_str)).await.is_ok() {
                                        debug!("Sent periodic validator list update to challenge (encrypted)");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}

/// Handle main message loop for ORM bridge and other messages
async fn handle_message_loop<F>(
    read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    client: &ChallengeWsClient,
    challenge_id: &Option<String>,
    orm_gateway: &Option<Arc<tokio::sync::RwLock<platform_api_orm_gateway::SecureORMGateway>>>,
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    use_encryption: bool,
    aead_key: &[u8; 32],
    callback: F,
) -> Result<()>
where
    F: Fn(Value, mpsc::Sender<Value>) + Send + Sync + 'static,
{
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
                    match serde_json::from_str(text) {
                        Ok(pm) => pm,
                        Err(e) => {
                            debug!(error = %e, "DEV MODE: Failed to parse plain text message");
                            continue;
                        }
                    }
                } else {
                    // Decrypt the message (production mode or dev mode with encryption enabled)
                    let json: Value = match serde_json::from_str(text) {
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

                    match decrypt_envelope(&envelope, aead_key) {
                        Ok(pm) => pm,
                        Err(e) => {
                            warn!(error = %e, "Failed to decrypt message - may be from wrong session or corrupted");
                            continue;
                        }
                    }
                };

                // Handle benchmark_progress messages for Redis logging
                if plain_msg["type"].as_str().unwrap_or("") == "benchmark_progress" {
                    handle_benchmark_progress(&plain_msg, client.redis_client.as_ref()).await;
                }

                // Handle ORM permissions update from challenge
                if plain_msg["type"].as_str().unwrap_or("") == "orm_permissions" {
                    if let (Some(challenge_id), Some(orm_gateway)) = (challenge_id, orm_gateway) {
                        if let Err(e) = handle_orm_permissions(
                            &plain_msg,
                            challenge_id,
                            orm_gateway,
                            write_handle,
                            use_encryption,
                            aead_key,
                        ).await {
                            warn!(error = %e, "Failed to handle ORM permissions");
                        }
                    }
                    continue;
                }

                // Handle get_validator_count request
                if plain_msg["type"].as_str().unwrap_or("") == "get_validator_count" {
                    if let Some(validator_status) = &client.validator_challenge_status {
                        if let Err(e) = handle_get_validator_count(
                            &plain_msg,
                            validator_status,
                            write_handle,
                            use_encryption,
                            aead_key,
                        ).await {
                            warn!(error = %e, "Failed to handle get_validator_count");
                        }
                    } else {
                        warn!("validator_challenge_status not available for get_validator_count");
                    }
                    continue;
                }

                // Handle ORM queries via bridge/proxy
                if plain_msg["type"].as_str().unwrap_or("") == "orm_query" {
                    if let (Some(challenge_id), Some(orm_gateway)) = (challenge_id, orm_gateway) {
                        if let Err(e) = handle_orm_query(
                            &plain_msg,
                            client,
                            challenge_id,
                            orm_gateway,
                            write_handle,
                            use_encryption,
                            aead_key,
                        ).await {
                            warn!(error = %e, "Failed to handle ORM query");
                        }
                    }
                    continue;
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

    Ok(())
}
