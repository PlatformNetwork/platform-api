//! Authentication handlers for WebSocket connections

use crate::state::AppState;
use anyhow::Result;
use futures_util::SinkExt;
use std::sync::Arc;
use tracing::{error, info, warn};

use super::super::auth::{
    compute_challenge_hash, verify_challenge_binding, verify_secure_message,
    verify_validator_attestation,
};
use super::super::messages::{AttestationMessage, HandshakeMessage, SecureMessage};
use super::super::utils::{
    extract_app_id_from_event_log, extract_compose_hash_from_event_log,
    extract_instance_id_from_event_log,
};

/// Handle unauthenticated messages (handshake and attestation)
pub async fn handle_unauthenticated_message(
    text: &str,
    hotkey: &str,
    state: &AppState,
    authenticated: &mut bool,
    awaiting_attestation: &mut bool,
    expected_challenge: &mut String,
    tx_clone: &Arc<tokio::sync::mpsc::Sender<String>>,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) -> Result<()> {
    // Check if it's a handshake
    if let Ok(handshake) = serde_json::from_str::<HandshakeMessage>(text) {
        if handshake.msg_type == "handshake" && !*awaiting_attestation {
            info!("Validator {} completed handshake", hotkey);
            // Generate challenge
            let challenge = compute_challenge_hash(hotkey);
            *expected_challenge = challenge.clone();

            let response = serde_json::json!({
                "type": "challenge",
                "challenge": challenge
            });

            let mut sender = sender_handle.lock().await;
            sender
                .send(axum::extract::ws::Message::Text(
                    serde_json::to_string(&response)?,
                ))
                .await?;
            *awaiting_attestation = true;
            return Ok(());
        }
    }

    // Check if it's an attestation message
    if *awaiting_attestation {
        if let Ok(attestation) = serde_json::from_str::<AttestationMessage>(text) {
            if attestation.msg_type == "attestation" {
                let dev_mode = std::env::var("DEV_MODE")
                    .unwrap_or_else(|_| "false".to_string())
                    .to_lowercase()
                    == "true"
                    || std::env::var("TEE_ENFORCED")
                        .unwrap_or_else(|_| "true".to_string())
                        .to_lowercase()
                        == "false";

                if dev_mode {
                    handle_dev_mode_attestation(
                        &attestation,
                        hotkey,
                        state,
                        authenticated,
                        expected_challenge,
                        tx_clone,
                        sender_handle,
                    )
                    .await?;
                } else {
                    handle_production_attestation(
                        &attestation,
                        hotkey,
                        state,
                        authenticated,
                        expected_challenge,
                        tx_clone,
                        sender_handle,
                    )
                    .await?;
                }
                return Ok(());
            }
        }
    }

    warn!("Unexpected message from unauthenticated validator: {}", text);
    Ok(())
}

/// Handle attestation in dev mode (no TDX verification)
async fn handle_dev_mode_attestation(
    attestation: &AttestationMessage,
    hotkey: &str,
    state: &AppState,
    authenticated: &mut bool,
    expected_challenge: &str,
    tx_clone: &Arc<tokio::sync::mpsc::Sender<String>>,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) -> Result<()> {
    info!("DEV MODE: Processing attestation for validator {}", hotkey);

    // Verify challenge binding
    if let Some(challenge_response) = &attestation.challenge_response {
        if !verify_challenge_binding(challenge_response, expected_challenge) {
            return Err(anyhow::anyhow!("Challenge binding verification failed"));
        }
    } else {
        return Err(anyhow::anyhow!("Missing challenge_response"));
    }

    // Extract compose_hash and instance_id from event log if present
    let compose_hash = attestation
        .event_log
        .as_ref()
        .and_then(|log| extract_compose_hash_from_event_log(log))
        .unwrap_or_else(|| "unknown".to_string());

    let instance_id = attestation
        .event_log
        .as_ref()
        .and_then(|log| extract_instance_id_from_event_log(log));

    let app_id = attestation
        .event_log
        .as_ref()
        .and_then(|log| extract_app_id_from_event_log(log));

    complete_authentication(
        hotkey,
        state,
        authenticated,
        &compose_hash,
        instance_id,
        app_id,
        tx_clone,
        sender_handle,
    )
    .await
}

/// Handle attestation in production mode (with TDX verification)
async fn handle_production_attestation(
    attestation: &AttestationMessage,
    hotkey: &str,
    state: &AppState,
    authenticated: &mut bool,
    expected_challenge: &str,
    tx_clone: &Arc<tokio::sync::mpsc::Sender<String>>,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) -> Result<()> {
    info!("Production mode: Processing attestation for validator {}", hotkey);

    // Verify challenge binding
    if let Some(challenge_response) = &attestation.challenge_response {
        if !verify_challenge_binding(challenge_response, expected_challenge) {
            return Err(anyhow::anyhow!("Challenge binding verification failed"));
        }
    } else {
        return Err(anyhow::anyhow!("Missing challenge_response"));
    }

    // Verify TDX attestation
    if let Some(quote) = &attestation.quote {
        verify_validator_attestation(quote, expected_challenge)?;
    } else {
        return Err(anyhow::anyhow!("Missing TDX quote in production mode"));
    }

    // Extract compose_hash and instance_id from event log
    let compose_hash = attestation
        .event_log
        .as_ref()
        .and_then(|log| extract_compose_hash_from_event_log(log))
        .ok_or_else(|| anyhow::anyhow!("Missing compose_hash in event_log"))?;

    let instance_id = attestation
        .event_log
        .as_ref()
        .and_then(|log| extract_instance_id_from_event_log(log));

    let app_id = attestation
        .event_log
        .as_ref()
        .and_then(|log| extract_app_id_from_event_log(log));

    complete_authentication(
        hotkey,
        state,
        authenticated,
        &compose_hash,
        instance_id,
        app_id,
        tx_clone,
        sender_handle,
    )
    .await
}

/// Complete authentication process
async fn complete_authentication(
    hotkey: &str,
    state: &AppState,
    authenticated: &mut bool,
    compose_hash: &str,
    instance_id: Option<String>,
    app_id: Option<String>,
    tx_clone: &Arc<tokio::sync::mpsc::Sender<String>>,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) -> Result<()> {
    info!(
        "Authentication successful for validator {} (compose_hash: {})",
        hotkey, compose_hash
    );

    // Store validator connection
    state
        .add_validator_connection(
            hotkey,
            compose_hash.to_string(),
            instance_id,
            app_id,
            tx_clone.clone(),
        )
        .await;

    // Send authentication success message
    let success_msg = serde_json::json!({
        "type": "authenticated",
        "message": "Authentication successful"
    });

    let mut sender = sender_handle.lock().await;
    sender
        .send(axum::extract::ws::Message::Text(
            serde_json::to_string(&success_msg)?,
        ))
        .await?;

    *authenticated = true;
    Ok(())
}

