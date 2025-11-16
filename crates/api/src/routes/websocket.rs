use anyhow::Context;
use axum::{
    extract::{ws::WebSocketUpgrade, Path, State},
    response::Response,
    Router,
};
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sp_core::{crypto::Ss58Codec, sr25519};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{AttestationRequest, AttestationType};

/// Create WebSocket router
pub fn create_router() -> Router<AppState> {
    Router::new().route(
        "/validators/:hotkey/ws",
        axum::routing::get(validator_websocket),
    )
}

/// WebSocket handler for validator connections
pub async fn validator_websocket(
    Path(hotkey): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    info!("Validator WebSocket connection request from: {}", hotkey);

    ws.on_upgrade(move |socket| handle_validator_socket(socket, hotkey, state))
}

async fn handle_validator_socket(
    socket: axum::extract::ws::WebSocket,
    hotkey: String,
    state: AppState,
) {
    info!("Validator WebSocket connected: {}", hotkey);

    let (mut sender, mut receiver) = socket.split();

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
                // Message received (logging removed for verbosity)

                // Before authentication, only accept handshake and attestation messages
                if !authenticated {
                    // Check if it's a handshake
                    if let Ok(handshake) = serde_json::from_str::<HandshakeMessage>(&text) {
                        if handshake.msg_type == "handshake" && !awaiting_attestation {
                            info!("Validator {} completed handshake", hotkey);

                            // Check if TEE is enforced - skip attestation in dev mode
                            let tee_enforced = std::env::var("TEE_ENFORCED")
                                .unwrap_or_else(|_| "true".to_string())
                                .to_lowercase()
                                == "true";

                            if !tee_enforced {
                                // Dev mode: Request mock attestation with event log containing compose_hash
                                // This ensures we still verify compose_hash even in dev mode
                                warn!(
                                    "DEV MODE: Requesting mock TDX attestation for validator {} (TDX verification will be mocked but compose_hash will be verified)",
                                    hotkey
                                );
                                awaiting_attestation = true;

                                // Generate random challenge for this session (still needed for mock attestation)
                                use rand::RngCore;
                                let mut challenge_bytes = [0u8; 32];
                                rand::thread_rng().fill_bytes(&mut challenge_bytes);
                                expected_challenge = hex::encode(challenge_bytes);

                                info!(
                                    "Generated challenge for validator {} (dev mode): {}",
                                    hotkey, expected_challenge
                                );

                                // Request mock TDX attestation with challenge
                                // Validator must provide event log with compose_hash even in dev mode
                                let attest_request = serde_json::json!({
                                    "type": "request_attestation",
                                    "message": "Please provide mock TDX attestation (dev mode - TDX verification will be mocked but compose_hash must be provided in event log)",
                                    "challenge": expected_challenge,
                                    "dev_mode": true
                                });

                                {
                                    let mut sender = sender_handle.lock().await;
                                    if let Err(e) = sender
                                        .send(axum::extract::ws::Message::Text(
                                            serde_json::to_string(&attest_request).unwrap(),
                                        ))
                                        .await
                                    {
                                        error!("Failed to send attestation request: {}", e);
                                        break;
                                    }
                                }

                                info!("Requested mock TDX attestation from validator {} (dev mode)", hotkey);
                                continue;
                            }

                            // Production mode: Request TDX attestation
                            awaiting_attestation = true;

                            // Generate random challenge for this session
                            use rand::RngCore;
                            let mut challenge_bytes = [0u8; 32];
                            rand::thread_rng().fill_bytes(&mut challenge_bytes);
                            expected_challenge = hex::encode(challenge_bytes);

                            info!(
                                "Generated challenge for validator {}: {}",
                                hotkey, expected_challenge
                            );

                            // Request TDX attestation with challenge
                            let attest_request = serde_json::json!({
                                "type": "request_attestation",
                                "message": "Please provide TDX attestation",
                                "challenge": expected_challenge
                            });

                            {
                                let mut sender = sender_handle.lock().await;
                                if let Err(e) = sender
                                    .send(axum::extract::ws::Message::Text(
                                        serde_json::to_string(&attest_request).unwrap(),
                                    ))
                                    .await
                                {
                                    error!("Failed to request attestation: {}", e);
                                    break;
                                }
                            }
                            continue;
                        }
                    }

                    // Check if it's a secure attestation response
                    if let Ok(secure_msg) = serde_json::from_str::<SecureMessage>(&text) {
                        if secure_msg.message_type == "attestation_response" && awaiting_attestation
                        {
                            info!("Received secure TDX attestation from validator {}", hotkey);

                            // Verify message signature first
                            match verify_secure_message(&secure_msg, &hotkey).await {
                                Ok(()) => {
                                    debug!("Message signature verified for validator {}", hotkey);
                                }
                                Err(e) => {
                                    error!(
                                        "Signature verification failed for validator {}: {}",
                                        hotkey, e
                                    );
                                    let ack = serde_json::json!({
                                        "type": "handshake_ack",
                                        "status": "failed",
                                        "error": format!("Signature verification failed: {}", e)
                                    });
                                    {
                                        let mut sender = sender_handle.lock().await;
                                        let _ = sender
                                            .send(axum::extract::ws::Message::Text(
                                                serde_json::to_string(&ack).unwrap(),
                                            ))
                                            .await;
                                    }
                                    break;
                                }
                            }

                            // Extract attestation data from secure message
                            let quote = match secure_msg.data["quote"].as_str() {
                                Some(q) => q,
                                None => {
                                    error!("Missing quote in secure message");
                                    break;
                                }
                            };
                            let event_log = match secure_msg.data["event_log"].as_str() {
                                Some(e) => e,
                                None => {
                                    error!("Missing event_log");
                                    break;
                                }
                            };
                            let received_challenge = match secure_msg.data["challenge"].as_str() {
                                Some(c) => c,
                                None => {
                                    error!("Missing challenge");
                                    break;
                                }
                            };

                            // Verify challenge matches expected challenge
                            if received_challenge != expected_challenge {
                                error!(
                                    "Challenge mismatch for validator {}: expected {}, got {}",
                                    hotkey, expected_challenge, received_challenge
                                );
                                let ack = serde_json::json!({
                                    "type": "handshake_ack",
                                    "status": "failed",
                                    "error": "Challenge mismatch - possible replay attack"
                                });
                                {
                                    let mut sender = sender_handle.lock().await;
                                    let _ = sender
                                        .send(axum::extract::ws::Message::Text(
                                            serde_json::to_string(&ack).unwrap(),
                                        ))
                                        .await;
                                }
                                break;
                            }

                            info!("✅ Challenge verified for validator {}", hotkey);

                            // Check if we're in dev mode
                            let tee_enforced = std::env::var("TEE_ENFORCED")
                                .unwrap_or_else(|_| "true".to_string())
                                .to_lowercase()
                                == "true";

                            if !tee_enforced {
                                // Dev mode: Mock TDX verification but still verify challenge binding and extract compose_hash
                                warn!("DEV MODE: Mocking TDX verification for validator {} (but still verifying challenge binding and extracting compose_hash)", hotkey);
                                
                                // CRITICAL: Verify that report_data in TDX quote contains the challenge (even in dev mode)
                                // Decode report_data directly from the quote bytes to avoid trusting relayed fields
                                // Quote can be in base64 (from validator) or hex (legacy)
                                let quote_bytes = match base64_engine.decode(quote) {
                                    Ok(b) => b,
                                    Err(_) => {
                                        // Try hex as fallback for legacy compatibility
                                        match hex::decode(quote) {
                                            Ok(b) => b,
                                            Err(e) => {
                                                error!("Failed to decode quote (tried base64 and hex): {}", e);
                                                break;
                                            }
                                        }
                                    }
                                };
                                // Try common offsets (supports minor layout differences)
                                let candidate_offsets: [usize; 3] = [568, 576, 584];
                                let mut report_data_prefix = String::new();
                                let mut found = false;
                                for off in candidate_offsets.iter() {
                                    if quote_bytes.len() >= *off + 32 {
                                        let rd = &quote_bytes[*off..*off + 32];
                                        let hex32 = hex::encode(rd);
                                        // Keep the first 32 bytes (64 hex chars)
                                        report_data_prefix = hex32.chars().take(64).collect::<String>();
                                        found = true;
                                        info!(
                                            "DEV MODE: Using report_data at offset {}: {}..",
                                            off,
                                            &report_data_prefix
                                                [..std::cmp::min(8, report_data_prefix.len())]
                                        );
                                        break;
                                    }
                                }
                                if !found {
                                    error!("Quote too short to contain report_data at known offsets");
                                    let ack = serde_json::json!({
                                        "type": "handshake_ack",
                                        "status": "failed",
                                        "error": "TDX quote missing report_data"
                                    });
                                    {
                                        let mut sender = sender_handle.lock().await;
                                        let _ = sender
                                            .send(axum::extract::ws::Message::Text(
                                                serde_json::to_string(&ack).unwrap(),
                                            ))
                                            .await;
                                    }
                                    break;
                                }
                                // The report_data must contain or match the challenge (even in dev mode)
                                let challenge_hash = {
                                    let mut hasher = Sha256::new();
                                    hasher.update(expected_challenge.as_bytes());
                                    hex::encode(hasher.finalize())
                                };
                                if report_data_prefix != expected_challenge
                                    && report_data_prefix != challenge_hash
                                {
                                    error!("❌ DEV MODE: TDX report_data does not contain challenge! Expected {}, got {}", 
                                        expected_challenge, report_data_prefix);
                                    let ack = serde_json::json!({
                                        "type": "handshake_ack",
                                        "status": "failed",
                                        "error": "TDX attestation not bound to session challenge"
                                    });
                                    {
                                        let mut sender = sender_handle.lock().await;
                                        let _ = sender
                                            .send(axum::extract::ws::Message::Text(
                                                serde_json::to_string(&ack).unwrap(),
                                            ))
                                            .await;
                                    }
                                    break;
                                }
                                info!("✅ DEV MODE: Challenge binding verified (TDX verification mocked)");

                                // Extract compose_hash from event log (required even in dev mode)
                                let compose_hash = match extract_compose_hash_from_event_log(event_log) {
                                    Ok(hash) => {
                                        info!("✅ DEV MODE: Extracted compose_hash from event log: {}", hash);
                                        hash
                                    }
                                    Err(e) => {
                                        error!("DEV MODE: Failed to extract compose_hash from event log: {}", e);
                                        let ack = serde_json::json!({
                                            "type": "handshake_ack",
                                            "status": "failed",
                                            "error": format!("Missing compose-hash in event log: {}", e)
                                        });
                                        {
                                            let mut sender = sender_handle.lock().await;
                                            let _ = sender
                                                .send(axum::extract::ws::Message::Text(
                                                    serde_json::to_string(&ack).unwrap(),
                                                ))
                                                .await;
                                        }
                                        break;
                                    }
                                };

                                // Continue with authentication using extracted compose_hash
                                authenticated = true;
                                awaiting_attestation = false;

                                // Extract app_id and instance_id from event log
                                let app_id = extract_app_id_from_event_log(event_log);
                                let instance_id = extract_instance_id_from_event_log(event_log);

                                // Save validator connection with message sender
                                let conn = crate::state::ValidatorConnection {
                                    validator_hotkey: hotkey.clone(),
                                    app_id,
                                    instance_id,
                                    compose_hash: compose_hash.clone(),
                                    connected_at: chrono::Utc::now(),
                                    session_token: "websocket".to_string(),
                                    last_ping: chrono::Utc::now(),
                                    message_sender: Some(tx_clone.clone()),
                                };
                                state.add_validator_connection(conn).await;

                                // Send success acknowledgment
                                let ack = serde_json::json!({
                                    "type": "handshake_ack",
                                    "status": "success",
                                    "compose_hash": compose_hash,
                                    "dev_mode": true,
                                    "message": "Authenticated (dev mode - TDX verification mocked but compose_hash verified)"
                                });

                                {
                                    let mut sender = sender_handle.lock().await;
                                    if let Err(e) = sender
                                        .send(axum::extract::ws::Message::Text(
                                            serde_json::to_string(&ack).unwrap(),
                                        ))
                                        .await
                                    {
                                        error!("Failed to send ack: {}", e);
                                        break;
                                    }
                                }

                                // Send challenge list after successful authentication
                                let challenges = state.list_challenges().await;
                                let challenges_msg = serde_json::json!({
                                    "type": "challenges:list",
                                    "challenges": challenges
                                });

                                {
                                    let mut sender = sender_handle.lock().await;
                                    if let Err(e) = sender
                                        .send(axum::extract::ws::Message::Text(
                                            serde_json::to_string(&challenges_msg).unwrap(),
                                        ))
                                        .await
                                    {
                                        error!("Failed to send challenge list: {}", e);
                                    }
                                }

                                info!("✅ Validator {} authenticated (dev mode - TDX mocked, compose_hash: {})", hotkey, compose_hash);
                                continue;
                            }

                            // Production mode: Full TDX verification
                            // CRITICAL: Verify that report_data in TDX quote contains the challenge
                            // Decode report_data directly from the quote bytes to avoid trusting relayed fields
                            // Quote can be in base64 (from validator) or hex (legacy)
                            let quote_bytes = match base64_engine.decode(quote) {
                                Ok(b) => b,
                                Err(_) => {
                                    // Try hex as fallback for legacy compatibility
                                    match hex::decode(quote) {
                                        Ok(b) => b,
                                        Err(e) => {
                                            error!("Failed to decode quote (tried base64 and hex): {}", e);
                                            break;
                                        }
                                    }
                                }
                            };
                            // Try common offsets (supports minor layout differences)
                            let candidate_offsets: [usize; 3] = [568, 576, 584];
                            let mut report_data_prefix = String::new();
                            let mut found = false;
                            for off in candidate_offsets.iter() {
                                if quote_bytes.len() >= *off + 32 {
                                    let rd = &quote_bytes[*off..*off + 32];
                                    let hex32 = hex::encode(rd);
                                    // Keep the first 32 bytes (64 hex chars)
                                    report_data_prefix = hex32.chars().take(64).collect::<String>();
                                    // We don't know yet if it matches; compute and check below
                                    found = true;
                                    info!(
                                        "Using report_data at offset {}: {}..",
                                        off,
                                        &report_data_prefix
                                            [..std::cmp::min(8, report_data_prefix.len())]
                                    );
                                    break;
                                }
                            }
                            if !found {
                                error!("Quote too short to contain report_data at known offsets");
                                let ack = serde_json::json!({
                                    "type": "handshake_ack",
                                    "status": "failed",
                                    "error": "TDX quote missing report_data"
                                });
                                {
                                    let mut sender = sender_handle.lock().await;
                                    let _ = sender
                                        .send(axum::extract::ws::Message::Text(
                                            serde_json::to_string(&ack).unwrap(),
                                        ))
                                        .await;
                                }
                                break;
                            }
                            // The report_data must contain or match the challenge
                            // TDX report_data is 64 bytes (128 hex), zero-padded. Compare first 32 bytes (64 hex chars)
                            let challenge_hash = {
                                let mut hasher = Sha256::new();
                                hasher.update(expected_challenge.as_bytes());
                                hex::encode(hasher.finalize())
                            };
                            if report_data_prefix != expected_challenge
                                && report_data_prefix != challenge_hash
                            {
                                error!("❌ TDX report_data does not contain challenge! Expected {}, got {}", 
                                    expected_challenge, report_data_prefix);
                                let ack = serde_json::json!({
                                    "type": "handshake_ack",
                                    "status": "failed",
                                    "error": "TDX attestation not bound to session challenge"
                                });
                                {
                                    let mut sender = sender_handle.lock().await;
                                    let _ = sender
                                        .send(axum::extract::ws::Message::Text(
                                            serde_json::to_string(&ack).unwrap(),
                                        ))
                                        .await;
                                }
                                break;
                            }

                            // Parse attestation data
                            let attestation_msg = AttestationMessage {
                                msg_type: "attestation".to_string(),
                                quote: Some(quote.to_string()),
                                event_log: Some(event_log.to_string()),
                                measurements: None,
                            };

                            // Verify TDX attestation (production mode - full verification)
                            match verify_validator_attestation(&state, &attestation_msg).await {
                                Ok(compose_hash) => {
                                    debug!("TDX attestation bound to session challenge for validator {}", hotkey);
                                    debug!(
                                        "Validator {} TDX verified. Compose hash: {}",
                                        hotkey, compose_hash
                                    );
                                    authenticated = true;
                                    awaiting_attestation = false;

                                    // Extract app_id and instance_id from event log
                                    let app_id = {
                                        if let Ok(event_log_json) =
                                            serde_json::from_str::<serde_json::Value>(event_log)
                                        {
                                            event_log_json.as_array().and_then(|events| {
                                                for event in events {
                                                    if let Some(event_type) =
                                                        event.get("event").and_then(|e| e.as_str())
                                                    {
                                                        if event_type == "app-id" {
                                                            return event
                                                                .get("event_payload")
                                                                .and_then(|p| p.as_str())
                                                                .map(|s| s.to_string());
                                                        }
                                                    }
                                                }
                                                None
                                            })
                                        } else {
                                            None
                                        }
                                    };

                                    let instance_id = {
                                        if let Ok(event_log_json) =
                                            serde_json::from_str::<serde_json::Value>(event_log)
                                        {
                                            event_log_json.as_array().and_then(|events| {
                                                for event in events {
                                                    if let Some(event_type) =
                                                        event.get("event").and_then(|e| e.as_str())
                                                    {
                                                        if event_type == "instance-id" {
                                                            return event
                                                                .get("event_payload")
                                                                .and_then(|p| p.as_str())
                                                                .map(|s| s.to_string());
                                                        }
                                                    }
                                                }
                                                None
                                            })
                                        } else {
                                            None
                                        }
                                    };

                                    // Save validator connection with message sender
                                    let conn = crate::state::ValidatorConnection {
                                        validator_hotkey: hotkey.clone(),
                                        app_id,
                                        instance_id,
                                        compose_hash: compose_hash.clone(),
                                        connected_at: chrono::Utc::now(),
                                        session_token: "websocket".to_string(), // WebSocket connection
                                        last_ping: chrono::Utc::now(),
                                        message_sender: Some(tx_clone.clone()),
                                    };
                                    state.add_validator_connection(conn).await;

                                    // Send success acknowledgment
                                    let ack = serde_json::json!({
                                        "type": "handshake_ack",
                                        "status": "success",
                                        "compose_hash": compose_hash
                                    });

                                    {
                                        let mut sender = sender_handle.lock().await;
                                        if let Err(e) = sender
                                            .send(axum::extract::ws::Message::Text(
                                                serde_json::to_string(&ack).unwrap(),
                                            ))
                                            .await
                                        {
                                            error!("Failed to send ack: {}", e);
                                        }
                                    }

                                    // Send challenge list after successful authentication
                                    let challenges = state.list_challenges().await;
                                    let challenges_msg = serde_json::json!({
                                        "type": "challenges:list",
                                        "challenges": challenges
                                    });

                                    {
                                        let mut sender = sender_handle.lock().await;
                                        if let Err(e) = sender
                                            .send(axum::extract::ws::Message::Text(
                                                serde_json::to_string(&challenges_msg).unwrap(),
                                            ))
                                            .await
                                        {
                                            error!("Failed to send challenge list: {}", e);
                                        }
                                    }

                                    continue;
                                }
                                Err(e) => {
                                    error!(
                                        "❌ TDX verification failed for validator {}: {}",
                                        hotkey, e
                                    );

                                    // Send failure acknowledgment
                                    let ack = serde_json::json!({
                                        "type": "handshake_ack",
                                        "status": "failed",
                                        "error": e.to_string()
                                    });

                                    {
                                        let mut sender = sender_handle.lock().await;
                                        if let Err(e) = sender
                                            .send(axum::extract::ws::Message::Text(
                                                serde_json::to_string(&ack).unwrap(),
                                            ))
                                            .await
                                        {
                                            error!("Failed to send error: {}", e);
                                        }
                                    }

                                    // Close connection
                                    break;
                                }
                            }
                        }
                    }

                    // Reject any other messages before authentication
                    warn!(
                        "Rejecting unauthenticated message from validator {}: {}",
                        hotkey, text
                    );
                    let reject_msg = serde_json::json!({
                        "type": "error",
                        "message": "Authentication required. Please complete handshake and TDX attestation."
                    });

                    {
                        let mut sender = sender_handle.lock().await;
                        if let Err(e) = sender
                            .send(axum::extract::ws::Message::Text(
                                serde_json::to_string(&reject_msg).unwrap(),
                            ))
                            .await
                        {
                            error!("Failed to send rejection: {}", e);
                        }
                    }
                    continue;
                }

                // After authentication, handle normal messages
                // (Logging removed for verbosity)

                // Parse and handle different message types
                if let Ok(msg_json) = serde_json::from_str::<serde_json::Value>(&text) {
                    let msg_type = msg_json["message_type"].as_str().unwrap_or("");
                    match msg_type {
                        "challenge_status" => {
                            // Handle validator challenge status update
                            match serde_json::from_value::<
                                Vec<platform_api_models::ValidatorChallengeStatus>,
                            >(msg_json["statuses"].clone())
                            {
                                Ok(statuses) => {
                                    let count = statuses.len();
                                    for status in statuses {
                                        state
                                            .update_validator_challenge_status(&hotkey, status)
                                            .await;
                                    }
                                    info!("Updated challenge status for validator {} ({} statuses)", hotkey, count);
                                }
                                Err(e) => {
                                    warn!("Failed to parse challenge_status from validator {}: {}. Raw statuses: {:?}", hotkey, e, msg_json.get("statuses"));
                                }
                            }
                        }
                        "orm_query" => {
                            // Handle ORM query from validator
                            // Use normal gateway to allow write operations based on permissions
                            if let (Some(orm_gateway), Some(query_data), Some(challenge_id)) =
                                (state.orm_gateway.as_ref(), msg_json.get("query"), msg_json.get("challenge_id").and_then(|v| v.as_str()))
                            {
                                // Extract query_id for response matching
                                let query_id = msg_json.get("query_id")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                
                                match handle_orm_query_with_challenge(&state, orm_gateway, query_data, challenge_id, &hotkey).await {
                                    Ok(result) => {
                                        let mut response = serde_json::json!({
                                            "type": "orm_result",
                                            "result": result
                                        });
                                        
                                        // Include query_id if present for matching
                                        if let Some(qid) = query_id {
                                            response["query_id"] = serde_json::Value::String(qid);
                                        }

                                        {
                                            let mut sender = sender_handle.lock().await;
                                            if let Err(e) = sender
                                                .send(axum::extract::ws::Message::Text(
                                                    serde_json::to_string(&response).unwrap(),
                                                ))
                                                .await
                                            {
                                                error!("Failed to send ORM result: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let mut error_response = serde_json::json!({
                                            "type": "error",
                                            "message": format!("ORM query failed: {}", e)
                                        });
                                        
                                        // Include query_id if present
                                        if let Some(qid) = query_id {
                                            error_response["query_id"] = serde_json::Value::String(qid);
                                        }

                                        {
                                            let mut sender = sender_handle.lock().await;
                                            if let Err(e) = sender
                                                .send(axum::extract::ws::Message::Text(
                                                    serde_json::to_string(&error_response).unwrap(),
                                                ))
                                                .await
                                            {
                                                error!("Failed to send ORM error: {}", e);
                                            }
                                        }
                                    }
                                }
                            } else {
                                let mut error_response = serde_json::json!({
                                    "type": "error",
                                    "message": "ORM gateway not available or missing required fields"
                                });
                                
                                // Try to extract query_id even if query is missing
                                let query_id = msg_json.get("query_id")
                                    .and_then(|v| v.as_str());
                                if let Some(qid) = query_id {
                                    error_response["query_id"] = serde_json::Value::String(qid.to_string());
                                }

                                {
                                    let mut sender = sender_handle.lock().await;
                                    if let Err(e) = sender
                                        .send(axum::extract::ws::Message::Text(
                                            serde_json::to_string(&error_response).unwrap(),
                                        ))
                                        .await
                                    {
                                        error!("Failed to send error: {}", e);
                                    }
                                }
                            }
                        }
                        "orm_permissions" => {
                            // Handle ORM permissions update
                            if let (Some(orm_gateway), Some(permissions), Some(challenge_id)) = (
                                state.orm_gateway.as_ref(),
                                msg_json.get("permissions"),
                                msg_json["challenge_id"].as_str(),
                            ) {
                                match handle_orm_permissions(orm_gateway, challenge_id, permissions)
                                    .await
                                {
                                    Ok(()) => {
                                        let response = serde_json::json!({
                                            "type": "orm_permissions_ack",
                                            "status": "success"
                                        });

                                        {
                                            let mut sender = sender_handle.lock().await;
                                            if let Err(e) = sender
                                                .send(axum::extract::ws::Message::Text(
                                                    serde_json::to_string(&response).unwrap(),
                                                ))
                                                .await
                                            {
                                                error!("Failed to send permissions ack: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let error_response = serde_json::json!({
                                            "type": "error",
                                            "message": format!("Failed to update permissions: {}", e)
                                        });

                                        {
                                            let mut sender = sender_handle.lock().await;
                                            if let Err(e) = sender
                                                .send(axum::extract::ws::Message::Text(
                                                    serde_json::to_string(&error_response).unwrap(),
                                                ))
                                                .await
                                            {
                                                error!("Failed to send error: {}", e);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        "job_result" => {
                            // Handle job result from validator and forward to challenge CVM
                            use crate::job_distributor::{JobDistributor, JobResult};

                            let job_id = msg_json
                                .get("job_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());

                            let result = msg_json
                                .get("result")
                                .cloned()
                                .unwrap_or(serde_json::Value::Null);

                            let error = msg_json
                                .get("error")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());

                            if let Some(job_id) = job_id {
                                let job_result = JobResult {
                                    job_id: job_id.clone(),
                                    result,
                                    error,
                                    validator_hotkey: Some(hotkey.clone()), // Include validator hotkey
                                };

                                let distributor = JobDistributor::new(state.clone());
                                match distributor.forward_job_result(job_result).await {
                                    Ok(_) => {
                                        info!(
                                            job_id = &job_id,
                                            validator_hotkey = &hotkey,
                                            "Job result forwarded to challenge CVM"
                                        );

                                        // Send acknowledgment to validator
                                        let ack = serde_json::json!({
                                            "type": "job_result_ack",
                                            "job_id": job_id,
                                            "status": "forwarded"
                                        });

                                        {
                                            let mut sender = sender_handle.lock().await;
                                            if let Err(e) = sender
                                                .send(axum::extract::ws::Message::Text(
                                                    serde_json::to_string(&ack).unwrap(),
                                                ))
                                                .await
                                            {
                                                error!("Failed to send job_result_ack: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!(
                                            job_id = &job_id,
                                            error = %e,
                                            "Failed to forward job result"
                                        );

                                        let error_response = serde_json::json!({
                                            "type": "job_result_error",
                                            "job_id": job_id,
                                            "message": format!("Failed to forward result: {}", e)
                                        });

                                        {
                                            let mut sender = sender_handle.lock().await;
                                            if let Err(e) = sender
                                                .send(axum::extract::ws::Message::Text(
                                                    serde_json::to_string(&error_response).unwrap(),
                                                ))
                                                .await
                                            {
                                                error!("Failed to send error response: {}", e);
                                            }
                                        }
                                    }
                                }
                            } else {
                                warn!("job_result message missing job_id");
                            }
                        }
                        _ => {
                            // Log other message types for monitoring (reduced verbosity)
                            // info!("Received message type: {} from validator {}", msg_type, hotkey);
                        }
                    }
                }
            }
            Ok(axum::extract::ws::Message::Close(_)) => {
                info!("Validator {} disconnected", hotkey);
                // Remove validator connection
                state.remove_validator_connection(&hotkey).await;
                break;
            }
            Err(e) => {
                error!("WebSocket error for validator {}: {}", hotkey, e);
                // Remove validator connection
                state.remove_validator_connection(&hotkey).await;
                break;
            }
            _ => {}
        }
    }

    info!("Validator WebSocket closed: {}", hotkey);
}

#[derive(Debug, Deserialize)]
struct HandshakeMessage {
    #[serde(rename = "type")]
    msg_type: String,
    validator_hotkey: String,
}

#[derive(Debug, Deserialize)]
struct AttestationMessage {
    #[serde(rename = "type")]
    msg_type: String,
    quote: Option<String>,
    event_log: Option<String>,
    measurements: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SecureMessage {
    pub message_type: String,
    pub data: serde_json::Value,
    pub timestamp: u64,
    pub nonce: String,
    pub signature: String,
    pub public_key: String,
}

/// Verify secure message signature and timestamp
async fn verify_secure_message(msg: &SecureMessage, expected_hotkey: &str) -> anyhow::Result<()> {
    // Verify timestamp is recent (within 30 seconds)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if now.saturating_sub(msg.timestamp) > 30 {
        return Err(anyhow::anyhow!(
            "Message timestamp too old: {} seconds",
            now.saturating_sub(msg.timestamp)
        ));
    }

    // Verify public key matches expected hotkey
    if msg.public_key != expected_hotkey {
        return Err(anyhow::anyhow!(
            "Public key mismatch: expected {}, got {}",
            expected_hotkey,
            msg.public_key
        ));
    }

    // Decode public key
    let public_key = sr25519::Public::from_ss58check(&msg.public_key)
        .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

    // Recreate message to verify
    let mut message = Vec::new();
    message.extend_from_slice(msg.message_type.as_bytes());
    message.extend_from_slice(msg.timestamp.to_string().as_bytes());
    message.extend_from_slice(msg.nonce.as_bytes());
    message.extend_from_slice(msg.data.to_string().as_bytes());

    // Decode signature
    let signature_bytes =
        hex::decode(&msg.signature).map_err(|e| anyhow::anyhow!("Invalid signature hex: {}", e))?;

    if signature_bytes.len() != 64 {
        return Err(anyhow::anyhow!("Invalid signature length"));
    }

    let mut sig_array = [0u8; 64];
    sig_array.copy_from_slice(&signature_bytes);
    let signature = sr25519::Signature::from(sig_array);

    // Verify signature using the verify_trait
    use sp_core::crypto::Pair;
    let is_valid = sr25519::Pair::verify(&signature, &message, &public_key);
    if !is_valid {
        return Err(anyhow::anyhow!("Signature verification failed"));
    }

    Ok(())
}

/// Extract compose_hash from event log
fn extract_compose_hash_from_event_log(event_log: &str) -> anyhow::Result<String> {
    let event_log_json: serde_json::Value =
        serde_json::from_str(event_log).context("Failed to parse event log")?;
    
    event_log_json
        .as_array()
        .and_then(|events| {
            for event in events {
                if let Some(event_type) = event.get("event").and_then(|e| e.as_str()) {
                    if event_type == "compose-hash" {
                        if let Some(payload) = event.get("event_payload").and_then(|p| p.as_str()) {
                            return Some(payload.to_string());
                        }
                    }
                }
            }
            None
        })
        .ok_or_else(|| anyhow::anyhow!("Missing compose-hash in event log"))
}

/// Extract app_id from event log
fn extract_app_id_from_event_log(event_log: &str) -> Option<String> {
    if let Ok(event_log_json) = serde_json::from_str::<serde_json::Value>(event_log) {
        event_log_json.as_array().and_then(|events| {
            for event in events {
                if let Some(event_type) = event.get("event").and_then(|e| e.as_str()) {
                    if event_type == "app-id" {
                        return event
                            .get("event_payload")
                            .and_then(|p| p.as_str())
                            .map(|s| s.to_string());
                    }
                }
            }
            None
        })
    } else {
        None
    }
}

/// Extract instance_id from event log
fn extract_instance_id_from_event_log(event_log: &str) -> Option<String> {
    if let Ok(event_log_json) = serde_json::from_str::<serde_json::Value>(event_log) {
        event_log_json.as_array().and_then(|events| {
            for event in events {
                if let Some(event_type) = event.get("event").and_then(|e| e.as_str()) {
                    if event_type == "instance-id" {
                        return event
                            .get("event_payload")
                            .and_then(|p| p.as_str())
                            .map(|s| s.to_string());
                    }
                }
            }
            None
        })
    } else {
        None
    }
}

/// Verify validator TDX attestation
async fn verify_validator_attestation(
    state: &AppState,
    msg: &AttestationMessage,
) -> anyhow::Result<String> {
    // Decode quote and event_log
    let quote = msg
        .quote
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing quote"))?;
    // Quote can be in base64 (from validator) or hex (legacy)
    let quote_bytes = match base64_engine.decode(quote) {
        Ok(b) => b,
        Err(_) => {
            // Try hex as fallback for legacy compatibility
            hex::decode(quote).context("Failed to decode quote (tried base64 and hex)")?
        }
    };

    let measurements = msg
        .measurements
        .as_ref()
        .map(|m| {
            m.iter()
                .map(|s| hex::decode(s).unwrap_or_default())
                .collect()
        })
        .unwrap_or_default();

    // Create attestation request
    let attest_request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(quote_bytes),
        report: None,
        nonce: vec![],
        measurements,
        capabilities: vec![],
    };

    // Verify attestation with event log
    let event_log = msg.event_log.as_deref();
    let result = state
        .attestation
        .verify_attestation_with_event_log(attest_request, event_log)
        .await
        .context("Failed to verify attestation")?;

    if !matches!(
        result.status,
        platform_api_models::AttestationStatus::Verified
    ) {
        return Err(anyhow::anyhow!(
            "Attestation verification failed: {:?}",
            result.error
        ));
    }

    // Extract compose_hash from event log TDX
    // CRITICAL: No fallback - if compose_hash is missing, reject attestation
    let event_log_str = msg
        .event_log
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing event log - cannot verify attestation"))?;

    // Parse event log JSON
    let event_log: serde_json::Value =
        serde_json::from_str(event_log_str).context("Failed to parse event log")?;

    // Extract compose_hash from event log
    // Look for event with type "compose-hash"
    let compose_hash = event_log
        .as_array()
        .and_then(|events| {
            for event in events {
                if let Some(event_type) = event.get("event").and_then(|e| e.as_str()) {
                    if event_type == "compose-hash" {
                        if let Some(payload) = event.get("event_payload").and_then(|p| p.as_str()) {
                            return Some(payload.to_string());
                        }
                    }
                }
            }
            None
        })
        .ok_or_else(|| {
            anyhow::anyhow!("Missing compose-hash in event log - attestation verification failed")
        })?;

    tracing::info!("Extracted compose_hash from event log: {}", compose_hash);

    Ok(compose_hash)
}

/// Handle ORM query request
async fn handle_orm_query(
    orm_gateway: &Arc<tokio::sync::RwLock<crate::orm_gateway::SecureORMGateway>>,
    query_data: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    use crate::orm_gateway::ORMQuery;

    // Parse the query
    let query: ORMQuery =
        serde_json::from_value(query_data.clone()).context("Failed to parse ORM query")?;

    // Execute the query
    let gateway = orm_gateway.read().await;
    let result = gateway
        .execute_read_query(query)
        .await
        .context("Failed to execute ORM query")?;

    // Convert result to JSON
    Ok(serde_json::to_value(result)?)
}

/// Handle ORM query from validator with challenge schema resolution
async fn handle_orm_query_with_challenge(
    state: &AppState,
    orm_gateway: &Arc<tokio::sync::RwLock<crate::orm_gateway::SecureORMGateway>>,
    query_data: &serde_json::Value,
    challenge_id: &str,
    validator_hotkey: &str,
) -> anyhow::Result<serde_json::Value> {
    use crate::orm_gateway::ORMQuery;

    // Parse the query
    let mut query: ORMQuery =
        serde_json::from_value(query_data.clone()).context("Failed to parse ORM query")?;

        // Resolve schema from ChallengeRunner if not provided
        if query.schema.is_none() {
            if let Some(challenge_runner) = &state.challenge_runner {
                if let Some(schema_name) = challenge_runner.get_schema_for_challenge(challenge_id).await {
                    query.schema = Some(schema_name.clone());
                } else {
                    // Fallback: challenge not managed by Platform API, use default format
                    warn!(
                        validator_hotkey = validator_hotkey,
                        challenge_id = challenge_id,
                        "Challenge not found in ChallengeRunner, using fallback schema format"
                    );
                    query.schema = Some(format!("challenge_{}", challenge_id.replace('-', "_")));
                }
            } else {
                // No ChallengeRunner available, use fallback
                query.schema = Some(format!("challenge_{}", challenge_id.replace('-', "_")));
            }
        }

    // Execute the query using normal gateway (permissions control access)
    let gateway = orm_gateway.read().await;
    let result = gateway
        .execute_query(query)
        .await
        .context("Failed to execute ORM query")?;

    // Convert result to JSON
    Ok(serde_json::to_value(result)?)
}

/// Handle ORM permissions update
async fn handle_orm_permissions(
    orm_gateway: &Arc<tokio::sync::RwLock<crate::orm_gateway::SecureORMGateway>>,
    challenge_id: &str,
    permissions_data: &serde_json::Value,
) -> anyhow::Result<()> {
    use crate::orm_gateway::TablePermission;
    use std::collections::HashMap;

    // Parse permissions
    let permissions: HashMap<String, TablePermission> =
        serde_json::from_value(permissions_data.clone())
            .context("Failed to parse ORM permissions")?;

    // Update permissions
    let mut gateway = orm_gateway.write().await;
    gateway
        .load_challenge_permissions(challenge_id, permissions)
        .await
        .context("Failed to load challenge permissions")?;

    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidatorNotification {
    pub msg_type: String,
    pub job_id: Option<Uuid>,
    pub challenge_id: Option<Uuid>,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handshake_message() {
        let json = r#"{"type":"handshake","validator_hotkey":"5DD123..."}"#;
        let msg: HandshakeMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type, "handshake");
    }
}
