//! Migration handling for WebSocket connections

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info};

use super::encryption::{encrypt_message, decrypt_envelope};
use super::types::{ChallengeWsClient, ConnectionState, EncryptedEnvelope};

/// Handle migrations during WebSocket connection
pub async fn handle_migrations(
    read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    conn_state: &mut ConnectionState,
    client: &ChallengeWsClient,
) -> Result<()> {
    info!("Starting migration process");

    // Send migration request
    let migration_request = serde_json::json!({
        "type": "migration_request"
    });

    let encrypted_msg = if let ConnectionState::Verified { key, .. } = conn_state {
        encrypt_message(&migration_request, key)?
    } else {
        return Err(anyhow!("Connection not verified for migration request"));
    };

    {
        let mut sender = write_handle.lock().await;
        sender
            .send(Message::Text(encrypted_msg))
            .await
            .context("Failed to send migration request")?;
    }

    // Wait for migration response
    process_migrations_response(read, write_handle, conn_state, client).await
}

/// Process migration response from challenge
pub async fn process_migrations_response(
    read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    conn_state: &mut ConnectionState,
    client: &ChallengeWsClient,
) -> Result<()> {
    loop {
        let msg = read
            .next()
            .await
            .ok_or_else(|| anyhow!("Connection closed during migration"))??;

        match msg {
            Message::Text(text) => {
                let envelope: EncryptedEnvelope = serde_json::from_str(&text)
                    .context("Failed to parse migration response envelope")?;

                if let ConnectionState::Verified { key, .. } = conn_state {
                    let decrypted = decrypt_envelope(&envelope, key)
                        .context("Failed to decrypt migration response")?;

                    let response: Value = serde_json::from_str(&decrypted)
                        .context("Failed to parse migration response JSON")?;

                    if let Some(typ) = response.get("type").and_then(|t| t.as_str()) {
                        match typ {
                            "migration_response" => {
                                info!("Migration response received");
                                // Handle migration completion
                                wait_for_migrations_applied(client).await;
                                send_orm_ready(write_handle, conn_state).await?;
                                return Ok(());
                            }
                            "migration_progress" => {
                                debug!("Migration progress: {:?}", response);
                                continue;
                            }
                            _ => {
                                debug!("Ignoring non-migration message during migration phase");
                                continue;
                            }
                        }
                    }
                } else {
                    return Err(anyhow!("Connection not verified when processing migration response"));
                }
            }
            Message::Close(_) => {
                return Err(anyhow!("Connection closed during migration"));
            }
            _ => {
                debug!("Ignoring non-text message during migration");
                continue;
            }
        }
    }
}

/// Wait for migrations to be applied
async fn wait_for_migrations_applied(client: &ChallengeWsClient) {
    info!("Waiting for migrations to be applied");
    // In a real implementation, you might want to poll or wait for a signal
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    info!("Migrations assumed to be applied");
}

/// Send ORM ready message
async fn send_orm_ready(
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    conn_state: &ConnectionState,
) -> Result<()> {
    let orm_ready = serde_json::json!({
        "type": "orm_ready"
    });

    let encrypted_msg = if let ConnectionState::Verified { key, .. } = conn_state {
        encrypt_message(&orm_ready, key)?
    } else {
        return Err(anyhow!("Connection not verified for ORM ready message"));
    };

    {
        let mut sender = write_handle.lock().await;
        sender
            .send(Message::Text(encrypted_msg))
            .await
            .context("Failed to send ORM ready message")?;
    }

    info!("Sent ORM ready message");
    Ok(())
}
