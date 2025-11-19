//! WebSocket connection management and main connection loop
//! Refactored version with modular structure

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit};
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use x25519_dalek::{EphemeralSecret, PublicKey};

use super::encryption::{encrypt_message, decrypt_envelope};
use super::types::{ChallengeWsClient, ConnectionState, EncryptedEnvelope};
use super::attestation::perform_attestation;
use super::migration_handler::{handle_migrations, wait_for_migrations_applied, send_orm_ready};
use super::message_loop::handle_message_loop;
use super::validator_manager::{send_initial_validator_list, spawn_validator_update_task};

/// Connect with reconnection logic and handle messages
/// Handles ORM queries from challenge SDK and executes them via ORM gateway
pub async fn connect_with_reconnect<F>(client: &ChallengeWsClient, callback: F) -> Result<()>
where
    F: Fn(Value, mpsc::Sender<Value>) + Send + Sync + 'static,
{
    info!("Connecting WebSocket to {}", client.url);

    // Establish WebSocket connection
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

    // Generate ephemeral X25519 key pair for attestation
    let api_secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let api_pub_bytes = api_secret.public_key().as_bytes().to_vec();
    let api_pub_b64 = base64_engine.encode(&api_pub_bytes);

    // Initial connection state
    let mut conn_state = ConnectionState::Unverified {
        started: Instant::now(),
        nonce: nonce_bytes,
    };

    // Send attestation request
    let attestation_request = serde_json::json!({
        "type": "attestation_request",
        "nonce": nonce_hex,
        "api_x25519_pub": api_pub_b64
    });

    {
        let mut sender = write_handle.lock().await;
        sender
            .send(Message::Text(attestation_request.to_string()))
            .await
            .context("Failed to send attestation request")?;
    }

    // Perform attestation handshake
    let key_bytes = perform_attestation(&mut read, &conn_state, api_secret, &write_handle).await?;
    
    // Update connection state to verified
    conn_state = ConnectionState::Verified {
        key: ChaCha20Poly1305::new_from_slice(&key_bytes)
            .map_err(|_| anyhow!("Failed to create encryption key"))?,
        compose_hash: client.compose_hash.clone(),
    };

    info!("✅ TDX attestation completed, connection verified");

    // Handle migrations
    handle_migrations(&mut read, &write_handle, &mut conn_state, client).await?;
    info!("✅ Migration process completed");

    // Send initial validator list
    send_initial_validator_list(&write_handle, &conn_state, &client.compose_hash).await?;

    // Spawn validator update task
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    spawn_validator_update_task(
        write_handle.clone(),
        conn_state.clone(),
        client.compose_hash.clone(),
        shutdown_rx,
    ).await?;

    // Enter main message loop
    let result = handle_message_loop(read, write_handle, conn_state, client.clone(), callback).await;

    // Shutdown validator update task
    let _ = shutdown_tx.send(());

    result
}

/// Reconnection logic with exponential backoff
pub async fn connect_with_backoff<F>(client: &ChallengeWsClient, callback: F) -> Result<()>
where
    F: Fn(Value, mpsc::Sender<Value>) + Send + Sync + 'static,
{
    let mut retry_count = 0;
    let max_retries = 5;
    let base_delay = tokio::time::Duration::from_secs(1);

    loop {
        match connect_with_reconnect(client, &callback).await {
            Ok(_) => {
                info!("WebSocket connection completed successfully");
                return Ok(());
            }
            Err(e) => {
                retry_count += 1;
                if retry_count >= max_retries {
                    error!("Max retry attempts ({}) reached, giving up", max_retries);
                    return Err(e);
                }

                let delay = base_delay * 2_u32.pow(retry_count - 1);
                warn!(
                    "WebSocket connection failed (attempt {}/{}): {}, retrying in {:?}",
                    retry_count, max_retries, e, delay
                );

                tokio::time::sleep(delay).await;
            }
        }
    }
}
