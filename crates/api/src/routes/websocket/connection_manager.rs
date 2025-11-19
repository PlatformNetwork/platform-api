//! WebSocket connection lifecycle management

use axum::extract::ws::WebSocket;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

use crate::state::AppState;

use super::authentication::{handle_unauthenticated_message, complete_authentication};
use super::utils::extract_compose_hash_from_event_log;

/// Main WebSocket connection handler
pub async fn handle_validator_connection(
    socket: WebSocket,
    hotkey: String,
    state: AppState,
) -> Result<(), anyhow::Error> {
    info!("Handling WebSocket connection for validator: {}", hotkey);

    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));

    // Create channel for sending messages to this validator
    let (tx, mut rx) = mpsc::channel::<String>(100);
    let tx_clone = Arc::new(tx);

    // Spawn task to forward messages from channel to WebSocket
    let sender_for_task = sender.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let mut sender = sender_for_task.lock().await;
            if let Err(e) = sender.send(axum::extract::ws::Message::Text(msg)).await {
                error!("Failed to send message to WebSocket: {}", e);
                break;
            }
        }
    });

    // Handle attestation phase
    let cipher = handle_attestation_phase(&mut receiver, &sender, &hotkey, &state).await?;

    // Complete authentication and switch to authenticated handling
    if let Some(cipher) = cipher {
        // Reconstruct WebSocket from parts for authenticated phase
        // Note: This is a simplified approach - in practice you might want to
        // keep the original split and pass the receiver directly
        complete_authentication(
            hotkey,
            cipher,
            sender.lock().await.clone(),
            receiver,
            state,
        ).await?;
    } else {
        warn!("Authentication failed for validator: {}", hotkey);
    }

    Ok(())
}

/// Handle the attestation phase of WebSocket connection
async fn handle_attestation_phase(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
    sender: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, axum::extract::ws::Message>>>,
    hotkey: &str,
    state: &AppState,
) -> Result<Option<chacha20poly1305::ChaCha20Poly1305>, anyhow::Error> {
    info!("Starting attestation phase for validator: {}", hotkey);

    // Set timeout for attestation phase
    let timeout = tokio::time::Duration::from_secs(30);
    let attestation_start = std::time::Instant::now();

    loop {
        // Check for timeout
        if attestation_start.elapsed() > timeout {
            error!("Attestation timeout for validator: {}", hotkey);
            return Ok(None);
        }

        tokio::select! {
            // Handle incoming messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(axum::extract::ws::Message::Text(text))) => {
                        match handle_unauthenticated_message(text, sender, hotkey, state).await {
                            Ok(Some(cipher)) => {
                                info!("✅ Attestation completed for validator: {}", hotkey);
                                return Ok(Some(cipher));
                            }
                            Ok(None) => {
                                // Continue waiting for attestation
                                continue;
                            }
                            Err(e) => {
                                error!("Attestation failed for validator {}: {}", hotkey, e);
                                return Ok(None);
                            }
                        }
                    }
                    Some(Ok(axum::extract::ws::Message::Close(_))) => {
                        warn!("WebSocket closed during attestation for validator: {}", hotkey);
                        return Ok(None);
                    }
                    Some(Ok(_)) => {
                        debug!("Ignoring non-text message during attestation for: {}", hotkey);
                        continue;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error during attestation for {}: {}", hotkey, e);
                        return Ok(None);
                    }
                    None => {
                        warn!("WebSocket stream ended during attestation for: {}", hotkey);
                        return Ok(None);
                    }
                }
            }
            // Handle timeout
            _ = tokio::time::sleep(timeout - attestation_start.elapsed()) => {
                error!("Attestation timeout for validator: {}", hotkey);
                return Ok(None);
            }
        }
    }
}

/// Setup periodic health checks for validator connections
pub async fn spawn_health_check_task(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        
        loop {
            interval.tick().await;
            
            if let Err(e) = perform_health_checks(&state).await {
                error!("Health check failed: {}", e);
            }
        }
    });
}

/// Perform health checks on all connected validators
async fn perform_health_checks(state: &AppState) -> Result<(), anyhow::Error> {
    let connections = state.get_all_validator_connections().await;
    let now = std::time::Instant::now();
    
    for (hotkey, connection) in connections {
        let time_since_heartbeat = now.duration_since(connection.last_heartbeat);
        
        // If no heartbeat for 5 minutes, send ping
        if time_since_heartbeat > std::time::Duration::from_secs(300) {
            debug!("Sending ping to inactive validator: {}", hotkey);
            
            let ping_msg = serde_json::json!({
                "type": "ping",
                "timestamp": chrono::Utc::now().timestamp()
            });
            
            if let Err(e) = connection.send_message(&ping_msg.to_string()).await {
                warn!("Failed to ping validator {}: {}", hotkey, e);
                
                // If ping fails multiple times, consider disconnecting
                if time_since_heartbeat > std::time::Duration::from_secs(600) {
                    warn!("Disconnecting inactive validator: {}", hotkey);
                    state.remove_validator_connection(&hotkey).await;
                }
            }
        }
    }
    
    Ok(())
}

/// Handle graceful shutdown of WebSocket connections
pub async fn shutdown_connections(state: AppState) {
    info!("Shutting down all WebSocket connections");
    
    let connections = state.get_all_validator_connections().await;
    
    for (hotkey, connection) in connections {
        let shutdown_msg = serde_json::json!({
            "type": "shutdown",
            "message": "Server is shutting down"
        });
        
        if let Err(e) = connection.send_message(&shutdown_msg.to_string()).await {
            error!("Failed to send shutdown message to {}: {}", hotkey, e);
        }
        
        // Give validators time to receive shutdown message
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        state.remove_validator_connection(&hotkey).await;
    }
    
    info!("All WebSocket connections shut down");
}
