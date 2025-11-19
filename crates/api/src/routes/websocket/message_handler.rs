//! WebSocket message handling for authenticated validators

use anyhow::{anyhow, Context, Result};
use axum::extract::ws::WebSocket;
use chacha20poly1305::ChaCha20Poly1305;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::state::AppState;

use super::messages::SecureMessage;
use super::encryption::{decrypt_message, encrypt_message};

/// Handle authenticated WebSocket messages
pub async fn handle_authenticated_messages(
    hotkey: String,
    mut receiver: futures_util::stream::SplitStream<WebSocket>,
    cipher: ChaCha20Poly1305,
    state: AppState,
) -> Result<()> {
    info!("Starting authenticated message handling for: {}", hotkey);

    loop {
        match receiver.next().await {
            Some(Ok(axum::extract::ws::Message::Text(text))) => {
                if let Err(e) = handle_authenticated_message(
                    &text,
                    &cipher,
                    &hotkey,
                    &state,
                ).await {
                    error!("Error handling authenticated message: {}", e);
                    // Continue processing other messages even if one fails
                }
            }
            Some(Ok(axum::extract::ws::Message::Close(close_frame))) => {
                info!("WebSocket closed for {}: {:?}", hotkey, close_frame);
                break;
            }
            Some(Ok(_)) => {
                debug!("Ignoring non-text message from: {}", hotkey);
                continue;
            }
            Some(Err(e)) => {
                error!("WebSocket error for {}: {}", hotkey, e);
                break;
            }
            None => {
                info!("WebSocket stream ended for: {}", hotkey);
                break;
            }
        }
    }

    // Unregister validator connection
    state.unregister_validator_connection(&hotkey).await;
    info!("Authenticated message handling ended for: {}", hotkey);

    Ok(())
}

/// Handle individual authenticated message
async fn handle_authenticated_message(
    text: &str,
    cipher: &ChaCha20Poly1305,
    hotkey: &str,
    state: &AppState,
) -> Result<()> {
    // Decrypt message
    let secure_msg: SecureMessage = serde_json::from_str(text)
        .context("Failed to parse secure message")?;

    let decrypted = decrypt_message(&secure_msg, cipher)
        .context("Failed to decrypt authenticated message")?;

    let msg_json: Value = serde_json::from_str(&decrypted)
        .context("Failed to parse decrypted message JSON")?;

    debug!("Received authenticated message from {}: {}", 
           hotkey, 
           msg_json.get("type").unwrap_or(&Value::Null));

    // Route message to appropriate handler
    if let Some(msg_type) = msg_json.get("type").and_then(|t| t.as_str()) {
        match msg_type {
            "challenge_status" => {
                handle_challenge_status(hotkey, &msg_json, state).await;
            }
            "orm_query" => {
                handle_orm_query(hotkey, &msg_json, cipher, state).await?;
            }
            "orm_permissions" => {
                handle_orm_permissions_msg(hotkey, &msg_json, cipher, state).await?;
            }
            "job_result" => {
                handle_job_result(hotkey, &msg_json, state).await?;
            }
            "heartbeat" => {
                handle_heartbeat(hotkey, state).await;
            }
            _ => {
                warn!("Unknown authenticated message type from {}: {}", hotkey, msg_type);
            }
        }
    }

    Ok(())
}

/// Handle challenge status updates
async fn handle_challenge_status(hotkey: &str, msg_json: &Value, state: &AppState) {
    debug!("Handling challenge status from {}: {:?}", hotkey, msg_json);

    if let Err(e) = state.update_validator_challenge_status(hotkey, msg_json).await {
        error!("Failed to update challenge status for {}: {}", hotkey, e);
    }
}

/// Handle ORM query requests
async fn handle_orm_query(
    hotkey: &str,
    msg_json: &Value,
    cipher: &ChaCha20Poly1305,
    state: &AppState,
) -> Result<()> {
    debug!("Handling ORM query from {}: {:?}", hotkey, msg_json);

    let result = super::orm::handle_orm_query_with_challenge(
        msg_json,
        hotkey,
        state,
    ).await;

    let response = match result {
        Ok(response) => response,
        Err(e) => {
            error!("ORM query failed for {}: {}", hotkey, e);
            serde_json::json!({
                "type": "orm_query_response",
                "success": false,
                "error": e.to_string()
            })
        }
    };

    // Send response back to validator
    if let Some(connection) = state.get_validator_connection(hotkey).await {
        let encrypted_response = encrypt_message(&response, cipher)?;
        
        if let Err(e) = connection.send_message(&encrypted_response).await {
            error!("Failed to send ORM query response to {}: {}", hotkey, e);
        }
    }

    Ok(())
}

/// Handle ORM permissions requests
async fn handle_orm_permissions_msg(
    hotkey: &str,
    msg_json: &Value,
    cipher: &ChaCha20Poly1305,
    state: &AppState,
) -> Result<()> {
    debug!("Handling ORM permissions from {}: {:?}", hotkey, msg_json);

    let result = super::orm::handle_orm_permissions(msg_json, hotkey, state).await;

    let response = match result {
        Ok(response) => response,
        Err(e) => {
            error!("ORM permissions check failed for {}: {}", hotkey, e);
            serde_json::json!({
                "type": "orm_permissions_response",
                "success": false,
                "error": e.to_string()
            })
        }
    };

    // Send response back to validator
    if let Some(connection) = state.get_validator_connection(hotkey).await {
        let encrypted_response = encrypt_message(&response, cipher)?;
        
        if let Err(e) = connection.send_message(&encrypted_response).await {
            error!("Failed to send ORM permissions response to {}: {}", hotkey, e);
        }
    }

    Ok(())
}

/// Handle job result submissions
async fn handle_job_result(
    hotkey: &str,
    msg_json: &Value,
    state: &AppState,
) -> Result<()> {
    debug!("Handling job result from {}: {:?}", hotkey, msg_json);

    if let Err(e) = state.process_job_result(hotkey, msg_json).await {
        error!("Failed to process job result for {}: {}", hotkey, e);
        return Err(e);
    }

    info!("Job result processed successfully for: {}", hotkey);
    Ok(())
}

/// Handle heartbeat messages
async fn handle_heartbeat(hotkey: &str, state: &AppState) {
    debug!("Received heartbeat from: {}", hotkey);

    if let Err(e) = state.update_validator_heartbeat(hotkey).await {
        error!("Failed to update heartbeat for {}: {}", hotkey, e);
    }
}
