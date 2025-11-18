use axum::{
    extract::{ws::WebSocketUpgrade, Path, State},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::state::AppState;

use super::auth::{
    compute_challenge_hash, verify_challenge_binding, verify_secure_message,
    verify_validator_attestation,
};
use super::messages::{AttestationMessage, HandshakeMessage, SecureMessage};
use super::orm::{handle_orm_permissions, handle_orm_query_with_challenge};
use super::utils::{
    extract_app_id_from_event_log, extract_compose_hash_from_event_log,
    extract_instance_id_from_event_log,
};

/// WebSocket handler for validator connections
pub async fn validator_websocket(
    Path(hotkey): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    info!("Validator WebSocket connection request from: {}", hotkey);

    ws.on_upgrade(move |socket| handle_validator_socket(socket, hotkey, state))
}

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

async fn handle_unauthenticated_message(
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
) -> anyhow::Result<()> {
    // Check if it's a handshake
    if let Ok(handshake) = serde_json::from_str::<HandshakeMessage>(text) {
        if handshake.msg_type == "handshake" && !*awaiting_attestation {
            info!("Validator {} completed handshake", hotkey);

            // Check if TEE is enforced - skip attestation in dev mode
            let tee_enforced = std::env::var("TEE_ENFORCED")
                .unwrap_or_else(|_| "true".to_string())
                .to_lowercase()
                == "true";

            *awaiting_attestation = true;

            // Generate random challenge for this session
            use rand::RngCore;
            let mut challenge_bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut challenge_bytes);
            *expected_challenge = hex::encode(challenge_bytes);

            info!(
                "Generated challenge for validator {}: {}",
                hotkey, expected_challenge
            );

            // Request attestation
            let attest_request = if tee_enforced {
                serde_json::json!({
                    "type": "request_attestation",
                    "message": "Please provide TDX attestation",
                    "challenge": expected_challenge
                })
            } else {
                warn!(
                    "DEV MODE: Requesting mock TDX attestation for validator {} (TDX verification will be mocked but compose_hash will be verified)",
                    hotkey
                );
                serde_json::json!({
                    "type": "request_attestation",
                    "message": "Please provide mock TDX attestation (dev mode - TDX verification will be mocked but compose_hash must be provided in event log)",
                    "challenge": expected_challenge,
                    "dev_mode": true
                })
            };

            let mut sender = sender_handle.lock().await;
            if let Err(e) = sender
                .send(axum::extract::ws::Message::Text(
                    serde_json::to_string(&attest_request).unwrap(),
                ))
                .await
            {
                error!("Failed to request attestation: {}", e);
                return Err(anyhow::anyhow!("Failed to send attestation request"));
            }
            return Ok(());
        }
    }

    // Check if it's a secure attestation response
    if let Ok(secure_msg) = serde_json::from_str::<SecureMessage>(text) {
        if secure_msg.message_type == "attestation_response" && *awaiting_attestation {
            info!("Received secure TDX attestation from validator {}", hotkey);

            // Verify message signature first
            verify_secure_message(&secure_msg, hotkey).await?;
            debug!("Message signature verified for validator {}", hotkey);

            // Extract attestation data from secure message
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
                let mut sender = sender_handle.lock().await;
                let _ = sender
                    .send(axum::extract::ws::Message::Text(
                        serde_json::to_string(&ack).unwrap(),
                    ))
                    .await;
                return Err(anyhow::anyhow!("Challenge mismatch"));
            }

            info!("✅ Challenge verified for validator {}", hotkey);

            // Check if we're in dev mode
            let tee_enforced = std::env::var("TEE_ENFORCED")
                .unwrap_or_else(|_| "true".to_string())
                .to_lowercase()
                == "true";

            // Verify challenge binding
            let challenge_hash = compute_challenge_hash(expected_challenge);

            if !tee_enforced {
                // Dev mode
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
                .await?;
            } else {
                // Production mode
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
                .await?;
            }

            return Ok(());
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

    let mut sender = sender_handle.lock().await;
    let _ = sender
        .send(axum::extract::ws::Message::Text(
            serde_json::to_string(&reject_msg).unwrap(),
        ))
        .await;
    Ok(())
}

async fn handle_dev_mode_attestation(
    hotkey: &str,
    quote: &str,
    event_log: &str,
    expected_challenge: &str,
    challenge_hash: &str,
    reported_report_data: &Option<String>,
    state: &AppState,
    authenticated: &mut bool,
    awaiting_attestation: &mut bool,
    tx_clone: &Arc<tokio::sync::mpsc::Sender<String>>,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) -> anyhow::Result<()> {
    warn!("DEV MODE: Mocking TDX verification for validator {} (but still verifying challenge binding and extracting compose_hash)", hotkey);

    // Verify report_data contains challenge
    if let Some(ref reported_rd) = reported_report_data {
        info!("DEV MODE: Using report_data from message: {}", reported_rd);

        if !verify_challenge_binding(reported_rd, expected_challenge, challenge_hash) {
            error!("❌ DEV MODE: TDX report_data does not contain challenge! Expected {} or {}, but not found", 
                expected_challenge, challenge_hash);
            let ack = serde_json::json!({
                "type": "handshake_ack",
                "status": "failed",
                "error": "TDX attestation not bound to session challenge"
            });
            let mut sender = sender_handle.lock().await;
            let _ = sender
                .send(axum::extract::ws::Message::Text(
                    serde_json::to_string(&ack).unwrap(),
                ))
                .await;
            return Err(anyhow::anyhow!("Challenge binding failed"));
        }
    } else {
        error!("❌ DEV MODE: Missing report_data in message");
        let ack = serde_json::json!({
            "type": "handshake_ack",
            "status": "failed",
            "error": "Missing report_data in attestation message"
        });
        let mut sender = sender_handle.lock().await;
        let _ = sender
            .send(axum::extract::ws::Message::Text(
                serde_json::to_string(&ack).unwrap(),
            ))
            .await;
        return Err(anyhow::anyhow!("Missing report_data"));
    }

    info!("✅ DEV MODE: Challenge binding verified (TDX verification mocked)");

    // Extract metadata and complete authentication
    complete_authentication(
        hotkey,
        event_log,
        state,
        authenticated,
        awaiting_attestation,
        tx_clone,
        sender_handle,
        true, // dev mode
    )
    .await
}

async fn handle_production_attestation(
    hotkey: &str,
    quote: &str,
    event_log: &str,
    vm_config: Option<String>,
    expected_challenge: &str,
    challenge_hash: &str,
    reported_report_data: &Option<String>,
    state: &AppState,
    authenticated: &mut bool,
    awaiting_attestation: &mut bool,
    tx_clone: &Arc<tokio::sync::mpsc::Sender<String>>,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) -> anyhow::Result<()> {
    // Verify report_data contains challenge
    if let Some(ref reported_rd) = reported_report_data {
        info!("Using report_data from dstack SDK: {}", reported_rd);

        if !verify_challenge_binding(reported_rd, expected_challenge, challenge_hash) {
            error!("❌ TDX report_data does not contain challenge! Expected {} or {}, reported report_data: {}", 
                expected_challenge, challenge_hash, reported_rd);
            let ack = serde_json::json!({
                "type": "handshake_ack",
                "status": "failed",
                "error": format!("TDX attestation not bound to session challenge. Report data: {}", reported_rd)
            });
            let mut sender = sender_handle.lock().await;
            let _ = sender
                .send(axum::extract::ws::Message::Text(
                    serde_json::to_string(&ack).unwrap(),
                ))
                .await;
            return Err(anyhow::anyhow!("Challenge binding failed"));
        }
    } else {
        error!("❌ Missing report_data in message");
        let ack = serde_json::json!({
            "type": "handshake_ack",
            "status": "failed",
            "error": "Missing report_data in attestation message"
        });
        let mut sender = sender_handle.lock().await;
        let _ = sender
            .send(axum::extract::ws::Message::Text(
                serde_json::to_string(&ack).unwrap(),
            ))
            .await;
        return Err(anyhow::anyhow!("Missing report_data"));
    }

    info!("✅ Challenge verified successfully (report_data matches)");

    // Parse attestation data
    let attestation_msg = AttestationMessage {
        msg_type: "attestation".to_string(),
        quote: Some(quote.to_string()),
        event_log: Some(event_log.to_string()),
        measurements: None,
        vm_config,
    };

    // Verify TDX attestation (production mode - full verification)
    // Pass the challenge as string bytes for nonce verification
    // The validator computes SHA256 of the challenge as a hex string, not as decoded bytes
    // This matches the behavior in platform/bins/validator/src/http_server.rs:910
    // where it does: hasher.update(nonce_str.as_bytes())
    let challenge_bytes = expected_challenge.as_bytes();
    verify_validator_attestation(state, &attestation_msg, Some(challenge_bytes)).await?;

    debug!(
        "TDX attestation bound to session challenge for validator {}",
        hotkey
    );

    // Complete authentication
    complete_authentication(
        hotkey,
        event_log,
        state,
        authenticated,
        awaiting_attestation,
        tx_clone,
        sender_handle,
        false, // production mode
    )
    .await
}

async fn complete_authentication(
    hotkey: &str,
    event_log: &str,
    state: &AppState,
    authenticated: &mut bool,
    awaiting_attestation: &mut bool,
    tx_clone: &Arc<tokio::sync::mpsc::Sender<String>>,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
    dev_mode: bool,
) -> anyhow::Result<()> {
    *authenticated = true;
    *awaiting_attestation = false;

    // Extract optional compose hash for observability
    let compose_hash = extract_compose_hash_from_event_log(event_log);
    if let Some(hash) = &compose_hash {
        info!(
            "{}Extracted compose_hash from event log: {}",
            if dev_mode { "DEV MODE: " } else { "" },
            hash
        );
    } else {
        warn!(
            "{}compose-hash missing from event log; continuing because trust relies on TDX attestation",
            if dev_mode { "DEV MODE: " } else { "" }
        );
    }

    // Extract app_id and instance_id from event log
    let app_id = extract_app_id_from_event_log(event_log);
    let instance_id = extract_instance_id_from_event_log(event_log);

    // Save validator connection with message sender
    let conn = crate::state::ValidatorConnection {
        validator_hotkey: hotkey.to_string(),
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
    let mut ack = serde_json::json!({
        "type": "handshake_ack",
        "status": "success"
    });

    if dev_mode {
        ack["dev_mode"] = serde_json::json!(true);
        ack["message"] = serde_json::json!(
            "Authenticated (dev mode - TDX verification mocked but challenge binding verified)"
        );
    }

    if let Some(hash) = &compose_hash {
        ack["compose_hash"] = serde_json::json!(hash);
    }

    {
        let mut sender = sender_handle.lock().await;
        if let Err(e) = sender
            .send(axum::extract::ws::Message::Text(
                serde_json::to_string(&ack).unwrap(),
            ))
            .await
        {
            error!("Failed to send ack: {}", e);
            return Err(anyhow::anyhow!("Failed to send acknowledgment"));
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

    if let Some(hash) = &compose_hash {
        info!(
            "✅ Validator {} authenticated {}(compose_hash: {})",
            hotkey,
            if dev_mode {
                "(dev mode - TDX mocked) "
            } else {
                ""
            },
            hash
        );
    } else {
        info!(
            "✅ Validator {} authenticated {}(compose_hash unavailable)",
            hotkey,
            if dev_mode {
                "(dev mode - TDX mocked) "
            } else {
                ""
            }
        );
    }

    Ok(())
}

async fn handle_authenticated_message(
    text: &str,
    hotkey: &str,
    state: &AppState,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) {
    // Parse and handle different message types
    if let Ok(msg_json) = serde_json::from_str::<serde_json::Value>(text) {
        let msg_type = msg_json["message_type"].as_str().unwrap_or("");
        match msg_type {
            "challenge_status" => {
                handle_challenge_status(hotkey, &msg_json, state).await;
            }
            "orm_query" => {
                handle_orm_query(hotkey, &msg_json, state, sender_handle).await;
            }
            "orm_permissions" => {
                handle_orm_permissions_msg(hotkey, &msg_json, state, sender_handle).await;
            }
            "job_result" => {
                handle_job_result(hotkey, &msg_json, state, sender_handle).await;
            }
            _ => {
                // Log other message types for monitoring (reduced verbosity)
                // info!("Received message type: {} from validator {}", msg_type, hotkey);
            }
        }
    }
}

async fn handle_challenge_status(hotkey: &str, msg_json: &serde_json::Value, state: &AppState) {
    match serde_json::from_value::<Vec<platform_api_models::ValidatorChallengeStatus>>(
        msg_json["statuses"].clone(),
    ) {
        Ok(statuses) => {
            let count = statuses.len();
            for status in statuses {
                state
                    .update_validator_challenge_status(hotkey, status)
                    .await;
            }
            info!(
                "Updated challenge status for validator {} ({} statuses)",
                hotkey, count
            );
        }
        Err(e) => {
            warn!(
                "Failed to parse challenge_status from validator {}: {}. Raw statuses: {:?}",
                hotkey,
                e,
                msg_json.get("statuses")
            );
        }
    }
}

async fn handle_orm_query(
    hotkey: &str,
    msg_json: &serde_json::Value,
    state: &AppState,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) {
    if let (Some(orm_gateway), Some(query_data), Some(challenge_id)) = (
        state.orm_gateway.as_ref(),
        msg_json.get("query"),
        msg_json.get("challenge_id").and_then(|v| v.as_str()),
    ) {
        // Extract query_id for response matching
        let query_id = msg_json
            .get("query_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        match handle_orm_query_with_challenge(state, orm_gateway, query_data, challenge_id, hotkey)
            .await
        {
            Ok(result) => {
                let mut response = serde_json::json!({
                    "type": "orm_result",
                    "result": result
                });

                // Include query_id if present for matching
                if let Some(qid) = query_id {
                    response["query_id"] = serde_json::Value::String(qid);
                }

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
            Err(e) => {
                let mut error_response = serde_json::json!({
                    "type": "error",
                    "message": format!("ORM query failed: {}", e)
                });

                // Include query_id if present
                if let Some(qid) = query_id {
                    error_response["query_id"] = serde_json::Value::String(qid);
                }

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
    } else {
        let mut error_response = serde_json::json!({
            "type": "error",
            "message": "ORM gateway not available or missing required fields"
        });

        // Try to extract query_id even if query is missing
        let query_id = msg_json.get("query_id").and_then(|v| v.as_str());
        if let Some(qid) = query_id {
            error_response["query_id"] = serde_json::Value::String(qid.to_string());
        }

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

async fn handle_orm_permissions_msg(
    hotkey: &str,
    msg_json: &serde_json::Value,
    state: &AppState,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) {
    if let (Some(orm_gateway), Some(permissions), Some(challenge_id)) = (
        state.orm_gateway.as_ref(),
        msg_json.get("permissions"),
        msg_json["challenge_id"].as_str(),
    ) {
        match handle_orm_permissions(orm_gateway, challenge_id, permissions).await {
            Ok(()) => {
                let response = serde_json::json!({
                    "type": "orm_permissions_ack",
                    "status": "success"
                });

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
            Err(e) => {
                let error_response = serde_json::json!({
                    "type": "error",
                    "message": format!("Failed to update permissions: {}", e)
                });

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

async fn handle_job_result(
    hotkey: &str,
    msg_json: &serde_json::Value,
    state: &AppState,
    sender_handle: &Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
) {
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
            validator_hotkey: Some(hotkey.to_string()),
        };

        let distributor = JobDistributor::new(state.clone());
        match distributor.forward_job_result(job_result).await {
            Ok(_) => {
                info!(
                    job_id = &job_id,
                    validator_hotkey = hotkey,
                    "Job result forwarded to challenge CVM"
                );

                // Send acknowledgment to validator
                let ack = serde_json::json!({
                    "type": "job_result_ack",
                    "job_id": job_id,
                    "status": "forwarded"
                });

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
    } else {
        warn!("job_result message missing job_id");
    }
}
