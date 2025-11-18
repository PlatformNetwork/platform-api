//! Message handlers for authenticated WebSocket connections

use crate::state::AppState;
use futures_util::SinkExt;
use std::sync::Arc;
use tracing::{error, info, warn};

use super::super::orm::{handle_orm_permissions, handle_orm_query_with_challenge};

/// Handle authenticated messages
pub async fn handle_authenticated_message(
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

/// Handle challenge status updates
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

/// Handle ORM query messages
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

/// Handle ORM permissions messages
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

/// Handle job result messages
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

