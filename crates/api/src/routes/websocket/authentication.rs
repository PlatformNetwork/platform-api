//! WebSocket authentication and attestation handling

use anyhow::{anyhow, Context, Result};
use axum::extract::ws::WebSocket;
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit};
use futures_util::{SinkExt, StreamExt};
use hkdf::Hkdf;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::state::AppState;

use super::messages::{AttestationMessage, HandshakeMessage, SecureMessage};
use super::utils::{
    extract_app_id_from_event_log, extract_compose_hash_from_event_log,
    extract_instance_id_from_event_log,
};

/// Handle unauthenticated WebSocket messages during attestation phase
pub async fn handle_unauthenticated_message(
    msg: String,
    sender: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, axum::extract::ws::Message>>>,
    hotkey: &str,
    state: &AppState,
) -> Result<Option<ChaCha20Poly1305>> {
    let msg_json: Value = serde_json::from_str(&msg)
        .context("Failed to parse unauthenticated message")?;

    if let Some(msg_type) = msg_json.get("type").and_then(|t| t.as_str()) {
        match msg_type {
            "attestation_request" => {
                let attestation: AttestationMessage = serde_json::from_value(msg_json)
                    .context("Failed to parse attestation request")?;

                if is_dev_mode() {
                    return handle_dev_mode_attestation(attestation, sender, hotkey).await;
                } else {
                    return handle_production_attestation(attestation, sender, hotkey, state).await;
                }
            }
            _ => {
                warn!("Received unexpected message type during attestation: {}", msg_type);
                send_error_response(sender, "Expected attestation_request").await?;
            }
        }
    }

    Ok(None)
}

/// Handle development mode attestation (simplified)
async fn handle_dev_mode_attestation(
    attestation: AttestationMessage,
    sender: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, axum::extract::ws::Message>>>,
    hotkey: &str,
) -> Result<Option<ChaCha20Poly1305>> {
    info!("DEV MODE: Skipping attestation for validator: {}", hotkey);

    // Generate dummy key for dev mode
    let mut key_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key_bytes);
    let cipher = ChaCha20Poly1305::new_from_slice(&key_bytes)
        .map_err(|_| anyhow!("Failed to create cipher key"))?;

    // Send attestation response
    let response = serde_json::json!({
        "type": "attestation_response",
        "status": "success",
        "dev_mode": true
    });

    {
        let mut sender = sender.lock().await;
        sender
            .send(axum::extract::ws::Message::Text(response.to_string()))
            .await
            .context("Failed to send dev mode attestation response")?;
    }

    info!("✅ Dev mode attestation completed for: {}", hotkey);
    Ok(Some(cipher))
}

/// Handle production mode attestation with TDX verification
async fn handle_production_attestation(
    attestation: AttestationMessage,
    sender: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, axum::extract::ws::Message>>>,
    hotkey: &str,
    state: &AppState,
) -> Result<Option<ChaCha20Poly1305>> {
    info!("Starting production attestation for validator: {}", hotkey);

    // Verify validator exists and is registered
    let validator = state
        .get_validator_by_hotkey(hotkey)
        .await
        .context("Validator not found")?;

    // Verify attestation nonce and challenge binding
    if let Err(e) = verify_attestation_data(&attestation, &validator).await {
        error!("Attestation verification failed for {}: {}", hotkey, e);
        send_error_response(sender, "Attestation verification failed").await?;
        return Err(e);
    }

    // Generate ephemeral key pair
    let api_secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let api_pub_bytes = api_secret.public_key().as_bytes().to_vec();
    let api_pub_b64 = base64_engine.encode(&api_pub_bytes);

    // Derive shared secret
    let chal_pub_bytes = base64_engine.decode(&attestation.chal_x25519_pub)
        .context("Invalid challenge public key encoding")?;
    if chal_pub_bytes.len() != 32 {
        return Err(anyhow!("Invalid challenge public key length"));
    }
    let chal_pub_slice: &[u8; 32] = chal_pub_bytes[..32]
        .try_into()
        .map_err(|_| anyhow!("Failed to convert challenge public key"))?;
    let chal_pub = PublicKey::from(*chal_pub_slice);

    let shared_secret = api_secret.diffie_hellman(&chal_pub);

    // Derive encryption key
    let hkdf = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut key_bytes = [0u8; 32];
    hkdf.expand(b"platform-api-validator-v1", &mut key_bytes)
        .map_err(|_| anyhow!("HKDF expansion failed"))?;

    let cipher = ChaCha20Poly1305::new_from_slice(&key_bytes)
        .map_err(|_| anyhow!("Failed to create cipher key"))?;

    // Send attestation response
    let response = serde_json::json!({
        "type": "attestation_response",
        "api_x25519_pub": api_pub_b64,
        "status": "success"
    });

    {
        let mut sender = sender.lock().await;
        sender
            .send(axum::extract::ws::Message::Text(response.to_string()))
            .await
            .context("Failed to send attestation response")?;
    }

    info!("✅ Production attestation completed for: {}", hotkey);
    Ok(Some(cipher))
}

/// Complete authentication process after successful attestation
pub async fn complete_authentication(
    hotkey: String,
    cipher: ChaCha20Poly1305,
    sender: futures_util::stream::SplitSink<WebSocket, axum::extract::ws::Message>,
    receiver: futures_util::stream::SplitStream<WebSocket>,
    state: AppState,
) -> Result<()> {
    info!("Authentication completed for validator: {}", hotkey);

    // Register validator connection
    let connection = crate::state::ValidatorConnection {
        hotkey: hotkey.clone(),
        sender: Arc::new(Mutex::new(sender)),
        last_heartbeat: std::time::Instant::now(),
    };

    state
        .register_validator_connection(hotkey.clone(), connection)
        .await
        .context("Failed to register validator connection")?;

    // Start authenticated message handling
    super::message_handler::handle_authenticated_messages(
        hotkey,
        receiver,
        cipher,
        state,
    ).await?;

    Ok(())
}

/// Verify attestation data against validator records
async fn verify_attestation_data(
    attestation: &AttestationMessage,
    validator: &platform_api_models::Validator,
) -> Result<()> {
    // Verify nonce format
    if attestation.nonce.len() != 64 {
        return Err(anyhow!("Invalid nonce length"));
    }

    // Verify challenge binding if present
    if let Some(challenge_binding) = &attestation.challenge_binding {
        super::auth::verify_challenge_binding(challenge_binding, validator).await?;
    }

    // Additional TDX quote verification can be added here
    if let Some(quote) = &attestation.quote {
        super::auth::verify_validator_attestation(quote, &attestation.nonce).await?;
    }

    Ok(())
}

/// Send error response to WebSocket
async fn send_error_response(
    sender: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, axum::extract::ws::Message>>>,
    error_message: &str,
) -> Result<()> {
    let response = serde_json::json!({
        "type": "error",
        "message": error_message
    });

    let mut sender = sender.lock().await;
    sender
        .send(axum::extract::ws::Message::Text(response.to_string()))
        .await
        .context("Failed to send error response")?;

    Ok(())
}

/// Check if running in development mode
fn is_dev_mode() -> bool {
    std::env::var("DEV_MODE")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true"
        || std::env::var("TEE_ENFORCED")
            .unwrap_or_else(|_| "true".to_string())
            .to_lowercase()
            == "false"
}
