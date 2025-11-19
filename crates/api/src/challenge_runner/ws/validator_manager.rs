//! Validator connection and list management

use anyhow::{Context, Result};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info};

use super::encryption::encrypt_message;
use super::types::ConnectionState;
use super::validators::get_active_validators_for_compose_hash;

/// Send initial validator list to challenge
pub async fn send_initial_validator_list(
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    conn_state: &ConnectionState,
    compose_hash: &str,
) -> Result<()> {
    let validators = get_active_validators_for_compose_hash(compose_hash).await?;
    
    let validator_list_msg = serde_json::json!({
        "type": "validator_list",
        "validators": validators
    });

    let encrypted_msg = if let ConnectionState::Verified { key, .. } = conn_state {
        encrypt_message(&validator_list_msg, key)?
    } else {
        return Err(anyhow!("Connection not verified for validator list"));
    };

    {
        let mut sender = write_handle.lock().await;
        sender
            .send(Message::Text(encrypted_msg))
            .await
            .context("Failed to send validator list")?;
    }

    info!("Sent initial validator list with {} validators", validators.len());
    Ok(())
}

/// Spawn background task to update validator list periodically
pub async fn spawn_validator_update_task(
    write_handle: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    conn_state: ConnectionState,
    compose_hash: String,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = update_validator_list(&write_handle, &conn_state, &compose_hash).await {
                        debug!("Failed to update validator list: {}", e);
                    }
                }
                _ = &mut shutdown_rx => {
                    info!("Validator update task shutting down");
                    break;
                }
            }
        }
    });

    Ok(())
}

/// Update validator list with current active validators
async fn update_validator_list(
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    conn_state: &ConnectionState,
    compose_hash: &str,
) -> Result<()> {
    let validators = get_active_validators_for_compose_hash(compose_hash).await?;
    
    let update_msg = serde_json::json!({
        "type": "validator_list_update",
        "validators": validators,
        "timestamp": chrono::Utc::now().timestamp()
    });

    if let ConnectionState::Verified { key, .. } = conn_state {
        if let Ok(encrypted_msg) = encrypt_message(&update_msg, key) {
            let mut sender = write_handle.lock().await;
            if let Err(e) = sender.send(Message::Text(encrypted_msg)).await {
                debug!("Failed to send validator list update: {}", e);
            } else {
                debug!("Sent validator list update with {} validators", validators.len());
            }
        }
    }

    Ok(())
}
