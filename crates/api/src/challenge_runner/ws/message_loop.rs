//! Main message loop handling for WebSocket connections

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use super::encryption::{decrypt_envelope, encrypt_message};
use super::messages::{
    handle_benchmark_progress, handle_get_validator_count, handle_orm_permissions, handle_orm_query,
};
use super::types::{ChallengeWsClient, ConnectionState, EncryptedEnvelope};

/// Handle the main message loop after attestation and migrations
pub async fn handle_message_loop<F>(
    mut read: futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    write_handle: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    conn_state: ConnectionState,
    client: ChallengeWsClient,
    callback: F,
) -> Result<()>
where
    F: Fn(Value, mpsc::Sender<Value>) + Send + Sync + 'static,
{
    info!("Entering main message loop");

    // Create channel for sending messages to callback
    let (callback_tx, mut callback_rx) = mpsc::channel::<Value>(100);
    let callback_tx_clone = callback_tx.clone();

    // Spawn task to handle callback responses
    let write_for_callback = write_handle.clone();
    let conn_state_for_callback = conn_state.clone();
    tokio::spawn(async move {
        while let Some(response) = callback_rx.recv().await {
            if let Err(e) = send_response(&write_for_callback, &conn_state_for_callback, response).await {
                error!("Failed to send callback response: {}", e);
            }
        }
    });

    // Main message processing loop
    loop {
        tokio::select! {
            // Handle incoming WebSocket messages
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = handle_incoming_message(&text, &conn_state, &callback_tx, &client).await {
                            error!("Error handling incoming message: {}", e);
                            break;
                        }
                    }
                    Some(Ok(Message::Close(close_frame))) => {
                        info!("WebSocket closed: {:?}", close_frame);
                        break;
                    }
                    Some(Ok(_)) => {
                        debug!("Ignoring non-text message");
                        continue;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        info!("WebSocket stream ended");
                        break;
                    }
                }
            }
            // Handle control messages (ping, etc.)
            _ = handle_control_messages(&write_handle, &conn_state) => {
                continue;
            }
        }
    }

    info!("Message loop ended");
    Ok(())
}

/// Handle incoming message from WebSocket
async fn handle_incoming_message(
    text: &str,
    conn_state: &ConnectionState,
    callback_tx: &mpsc::Sender<Value>,
    client: &ChallengeWsClient,
) -> Result<()> {
    let envelope: EncryptedEnvelope = serde_json::from_str(text)
        .context("Failed to parse envelope")?;

    if let ConnectionState::Verified { key, .. } = conn_state {
        let decrypted = decrypt_envelope(&envelope, key)
            .context("Failed to decrypt message")?;

        let message: Value = serde_json::from_str(&decrypted)
            .context("Failed to parse message JSON")?;

        debug!("Received message: {}", message.get("type").unwrap_or(&Value::Null));

        // Route message to appropriate handler
        if let Some(msg_type) = message.get("type").and_then(|t| t.as_str()) {
            match msg_type {
                "orm_query" => {
                    handle_orm_query(message, callback_tx.clone()).await?;
                }
                "orm_permissions" => {
                    handle_orm_permissions(message, callback_tx.clone()).await?;
                }
                "get_validator_count" => {
                    handle_get_validator_count(message, callback_tx.clone(), client).await?;
                }
                "benchmark_progress" => {
                    handle_benchmark_progress(message, callback_tx.clone()).await?;
                }
                _ => {
                    warn!("Unknown message type: {}", msg_type);
                }
            }
        }
    } else {
        return Err(anyhow!("Connection not verified for message processing"));
    }

    Ok(())
}

/// Send response message through WebSocket
async fn send_response(
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    conn_state: &ConnectionState,
    response: Value,
) -> Result<()> {
    if let ConnectionState::Verified { key, .. } = conn_state {
        let encrypted_msg = encrypt_message(&response, key)?;
        
        let mut sender = write_handle.lock().await;
        sender
            .send(Message::Text(encrypted_msg))
            .await
            .context("Failed to send response")?;
    } else {
        return Err(anyhow!("Connection not verified for sending response"));
    }

    Ok(())
}

/// Handle control messages (ping, etc.)
async fn handle_control_messages(
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    conn_state: &ConnectionState,
) -> Result<()> {
    // Send periodic ping or other control messages
    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
    
    if let ConnectionState::Verified { key, .. } = conn_state {
        let ping_msg = serde_json::json!({
            "type": "ping",
            "timestamp": chrono::Utc::now().timestamp()
        });

        if let Ok(encrypted_msg) = encrypt_message(&ping_msg, key) {
            let mut sender = write_handle.lock().await;
            if let Err(e) = sender.send(Message::Text(encrypted_msg)).await {
                error!("Failed to send ping: {}", e);
            }
        }
    }

    Ok(())
}
