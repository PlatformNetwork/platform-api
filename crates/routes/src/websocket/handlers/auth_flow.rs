//! Authentication flow handlers

use anyhow::Result;
use futures_util::SinkExt;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use platform_api::state::AppState;

use crate::websocket::auth::{
    compute_challenge_hash, verify_challenge_binding, verify_secure_message,
    verify_validator_attestation,
};
use crate::websocket::messages::{AttestationMessage, HandshakeMessage, SecureMessage};

use super::attestation::{handle_dev_mode_attestation, handle_production_attestation};
use super::types::{MessageSender, WsSender};

/// Handle messages before authentication is complete
pub async fn handle_unauthenticated_message(
    text: &str,
    hotkey: &str,
    state: &AppState,
    authenticated: &mut bool,
    awaiting_attestation: &mut bool,
    expected_challenge: &mut String,
    tx_clone: &MessageSender,
    sender_handle: &WsSender,
) -> Result<()> {
    // Check if it's a handshake
    if let Ok(handshake) = serde_json::from_str::<HandshakeMessage>(text) {
        if handshake.msg_type == "handshake" && !*awaiting_attestation {
            return handle_handshake(
                hotkey,
                awaiting_attestation,
                expected_challenge,
                sender_handle,
            )
            .await;
        }
    }

    // Check if it's a secure attestation response
    if let Ok(secure_msg) = serde_json::from_str::<SecureMessage>(text) {
        if secure_msg.message_type == "attestation_response" && *awaiting_attestation {
            return handle_attestation_response(
                &secure_msg,
                hotkey,
                state,
                expected_challenge,
                authenticated,
                awaiting_attestation,
                tx_clone,
                sender_handle,
            )
            .await;
        }
    }

    // Reject any other messages before authentication
    warn!("Rejecting unauthenticated message from validator {}", hotkey);
    let reject_msg = serde_json::json!({
        "type": "error",
        "message": "Authentication required. Please complete handshake and TDX attestation."
    });

    let mut sender = sender_handle.lock().await;
    let _ = sender
        .send(axum::extract::ws::Message::Text(
            serde_json::to_string(&reject_msg).unwrap(),
        ))
        .await;
    Ok(())
}

/// Handle initial handshake
async fn handle_handshake(
    hotkey: &str,
    awaiting_attestation: &mut bool,
    expected_challenge: &mut String,
    sender_handle: &WsSender,
) -> Result<()> {
    info!("Validator {} completed handshake", hotkey);

    let tee_enforced = std::env::var("TEE_ENFORCED")
        .unwrap_or_else(|_| "true".to_string())
        .to_lowercase()
        == "true";

    *awaiting_attestation = true;

    // Generate random challenge
    use rand::RngCore;
    let mut challenge_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut challenge_bytes);
    *expected_challenge = hex::encode(challenge_bytes);

    info!("Generated challenge for validator {}: {}", hotkey, expected_challenge);

    // Request attestation
    let attest_request = if tee_enforced {
        serde_json::json!({
            "type": "request_attestation",
            "message": "Please provide TDX attestation",
            "challenge": expected_challenge
        })
    } else {
        warn!("DEV MODE: Requesting mock TDX attestation for validator {}", hotkey);
        serde_json::json!({
            "type": "request_attestation",
            "message": "Please provide mock TDX attestation (dev mode)",
            "challenge": expected_challenge,
            "dev_mode": true
        })
    };

    let mut sender = sender_handle.lock().await;
    sender
        .send(axum::extract::ws::Message::Text(
            serde_json::to_string(&attest_request).unwrap(),
        ))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send attestation request: {}", e))?;

    Ok(())
}

/// Handle attestation response
async fn handle_attestation_response(
    secure_msg: &SecureMessage,
    hotkey: &str,
    state: &AppState,
    expected_challenge: &str,
    authenticated: &mut bool,
    awaiting_attestation: &mut bool,
    tx_clone: &MessageSender,
    sender_handle: &WsSender,
) -> Result<()> {
    info!("Received secure TDX attestation from validator {}", hotkey);

    // Verify message signature
    verify_secure_message(secure_msg, hotkey).await?;
    debug!("Message signature verified for validator {}", hotkey);

    // Extract attestation data
    let quote = secure_msg.data["quote"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing quote in secure message"))?;
    let event_log = secure_msg.data["event_log"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing event_log"))?;
    let vm_config = secure_msg.data["vm_config"].as_str().map(|s| s.to_string());
    let received_challenge = secure_msg.data["challenge"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing challenge"))?;
    let reported_report_data = secure_msg.data["report_data"]
        .as_str()
        .map(|s| s.to_string());

    // Verify challenge matches
    if received_challenge != expected_challenge {
        error!("Challenge mismatch for validator {}", hotkey);
        let ack = serde_json::json!({
            "type": "handshake_ack",
            "status": "failed",
            "error": "Challenge mismatch"
        });
        let mut sender = sender_handle.lock().await;
        let _ = sender
            .send(axum::extract::ws::Message::Text(
                serde_json::to_string(&ack).unwrap(),
            ))
            .await;
        return Err(anyhow::anyhow!("Challenge mismatch"));
    }

    info!("✅ Challenge verified for validator {}", hotkey);

    // Check mode and verify
    let tee_enforced = std::env::var("TEE_ENFORCED")
        .unwrap_or_else(|_| "true".to_string())
        .to_lowercase()
        == "true";

    let challenge_hash = compute_challenge_hash(expected_challenge.to_string());

    if !tee_enforced {
        handle_dev_mode_attestation(
            hotkey,
            quote,
            event_log,
            expected_challenge,
            &challenge_hash,
            &reported_report_data,
            state,
            authenticated,
            awaiting_attestation,
            tx_clone,
            sender_handle,
        )
        .await
    } else {
        handle_production_attestation(
            hotkey,
            quote,
            event_log,
            vm_config,
            expected_challenge,
            &challenge_hash,
            &reported_report_data,
            state,
            authenticated,
            awaiting_attestation,
            tx_clone,
            sender_handle,
        )
        .await
    }
}

