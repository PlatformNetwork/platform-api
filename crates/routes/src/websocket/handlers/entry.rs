//! WebSocket entry point and main socket handler

use axum::{
    extract::{ws::WebSocketUpgrade, Path, State},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{error, info};

use platform_api::state::AppState;

use super::auth_flow::handle_unauthenticated_message;
use super::authenticated::handle_authenticated_message;

/// WebSocket handler for validator connections
pub async fn validator_websocket(
    Path(hotkey): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    info!("Validator WebSocket connection request from: {}", hotkey);

    ws.on_upgrade(move |socket| handle_validator_socket(socket, hotkey, state))
}

/// Handle validator WebSocket connection
pub async fn handle_validator_socket(
    socket: axum::extract::ws::WebSocket,
    hotkey: String,
    state: AppState,
) {
    info!("Validator WebSocket connected: {}", hotkey);

    let (sender, mut receiver) = socket.split();

    // Create channel for sending messages to this validator
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);
    let tx_clone = Arc::new(tx);

    // Wrap sender in Arc<Mutex> to share between tasks
    let sender_handle = Arc::new(tokio::sync::Mutex::new(sender));

    // Spawn task to forward messages from channel to WebSocket
    let sender_for_task = sender_handle.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let mut sender = sender_for_task.lock().await;
            if let Err(e) = sender.send(axum::extract::ws::Message::Text(msg)).await {
                error!("Failed to send message to validator: {}", e);
                break;
            }
        }
    });

    // Authentication state
    let mut authenticated = false;
    let mut awaiting_attestation = false;
    let mut expected_challenge = String::new();

    // Send welcome message
    let welcome_msg = serde_json::json!({
        "type": "welcome",
        "validator_hotkey": hotkey,
        "message": "Connected to Platform API"
    });

    {
        let mut sender = sender_handle.lock().await;
        if let Err(e) = sender
            .send(axum::extract::ws::Message::Text(
                serde_json::to_string(&welcome_msg).unwrap(),
            ))
            .await
        {
            error!("Failed to send welcome message: {}", e);
            return;
        }
    }

    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(axum::extract::ws::Message::Text(text)) => {
                // Before authentication, only accept handshake and attestation messages
                if !authenticated {
                    if let Err(e) = handle_unauthenticated_message(
                        &text,
                        &hotkey,
                        &state,
                        &mut authenticated,
                        &mut awaiting_attestation,
                        &mut expected_challenge,
                        &tx_clone,
                        &sender_handle,
                    )
                    .await
                    {
                        error!("Authentication error: {}", e);
                        break;
                    }
                    continue;
                }

                // After authentication, handle normal messages
                handle_authenticated_message(&text, &hotkey, &state, &sender_handle).await;
            }
            Ok(axum::extract::ws::Message::Close(_)) => {
                info!("Validator {} disconnected", hotkey);
                state.remove_validator_connection(&hotkey).await;
                break;
            }
            Err(e) => {
                error!("WebSocket error for validator {}: {}", hotkey, e);
                state.remove_validator_connection(&hotkey).await;
                break;
            }
            _ => {}
        }
    }

    info!("Validator WebSocket closed: {}", hotkey);
}

