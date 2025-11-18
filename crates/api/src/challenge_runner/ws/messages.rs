//! Message handling for WebSocket communication

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit};
use futures_util::SinkExt;
use rand::RngCore;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use super::encryption::{decrypt_envelope, encrypt_message};
use super::types::{ChallengeWsClient, EncryptedEnvelope};
use crate::redis_client::{create_job_log, create_job_progress};

/// Handle benchmark_progress messages for Redis logging
pub async fn handle_benchmark_progress(
    plain_msg: &Value,
    redis_client: Option<&Arc<crate::redis_client::RedisClient>>,
) {
    if let Some(redis) = redis_client {
        if let Some(job_id) = plain_msg
            .get("payload")
            .and_then(|p| p.get("job_id"))
            .and_then(|v| v.as_str())
        {
            if let Some(progress_data) = plain_msg.get("payload").and_then(|p| p.get("progress")) {
                // Extract progress metrics
                let progress_obj = progress_data.as_object();
                let progress_percent = progress_obj
                    .and_then(|p| p.get("progress_percent"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let total_tasks = progress_obj
                    .and_then(|p| p.get("total_tasks"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                let completed_tasks = progress_obj
                    .and_then(|p| p.get("completed_tasks"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                let resolved_tasks = progress_obj
                    .and_then(|p| p.get("resolved_tasks"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                let unresolved_tasks = progress_obj
                    .and_then(|p| p.get("unresolved_tasks"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);

                let status = progress_obj
                    .and_then(|p| p.get("status"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("running")
                    .to_string();

                // Log progress to Redis
                let progress = create_job_progress(
                    job_id.to_string(),
                    status,
                    progress_percent,
                    total_tasks,
                    completed_tasks,
                    resolved_tasks,
                    unresolved_tasks,
                    None,
                );

                if let Err(e) = redis.set_job_progress(&progress).await {
                    warn!("Failed to log job progress to Redis: {}", e);
                }

                // Log progress event to Redis logs
                let log_entry = create_job_log(
                    "info".to_string(),
                    format!(
                        "Progress update: {:.1}% ({} tasks completed)",
                        progress_percent,
                        completed_tasks.unwrap_or(0)
                    ),
                    Some(progress_data.clone()),
                );

                if let Err(e) = redis.append_job_log(job_id, &log_entry).await {
                    warn!("Failed to append job log to Redis: {}", e);
                }
            }
        }
    }
}

/// Handle ORM permissions update from challenge
pub async fn handle_orm_permissions(
    plain_msg: &Value,
    challenge_id: &str,
    orm_gateway: &Arc<tokio::sync::RwLock<platform_api_orm_gateway::SecureORMGateway>>,
    write_handle: &Arc<Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    use_encryption: bool,
    aead_key: &[u8; 32],
) -> Result<()> {
    // Extract permissions from payload
    if let (Some(permissions_json), Some(msg_challenge_id)) = (
        plain_msg.get("payload").and_then(|p| p.get("permissions")),
        plain_msg
            .get("payload")
            .and_then(|p| p.get("challenge_id"))
            .and_then(|v| v.as_str()),
    ) {
        // Verify challenge_id matches
        if msg_challenge_id == challenge_id {
            info!(
                challenge_id = challenge_id,
                "Received ORM permissions from challenge"
            );

            // Parse and load permissions
            match serde_json::from_value::<
                std::collections::HashMap<
                    String,
                    platform_api_orm_gateway::TablePermission,
                >,
            >(
                permissions_json.clone()
            ) {
                Ok(permissions) => {
                    let mut gateway = orm_gateway.write().await;
                    if let Err(e) = gateway
                        .load_challenge_permissions(challenge_id, permissions)
                        .await
                    {
                        error!(
                            challenge_id = challenge_id,
                            error = %e,
                            "Failed to load ORM permissions"
                        );
                    } else {
                        info!(
                            challenge_id = challenge_id,
                            "✅ ORM permissions loaded successfully"
                        );

                        // Send acknowledgment
                        let ack_msg = serde_json::json!({
                            "type": "orm_permissions_ack",
                            "status": "success"
                        });

                        send_message(write_handle, &ack_msg, use_encryption, aead_key).await?;
                    }
                }
                Err(e) => {
                    error!(
                        challenge_id = challenge_id,
                        error = %e,
                        "Failed to parse ORM permissions"
                    );
                }
            }
        } else {
            warn!(
                challenge_id = challenge_id,
                msg_challenge_id = msg_challenge_id,
                "ORM permissions challenge_id mismatch"
            );
        }
    } else {
        warn!("ORM permissions message missing required fields");
    }
    Ok(())
}

/// Handle get_validator_count request
pub async fn handle_get_validator_count(
    plain_msg: &Value,
    validator_status: &Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<
                String,
                std::collections::HashMap<
                    String,
                    platform_api_models::ValidatorChallengeStatus,
                >,
            >,
        >,
    >,
    write_handle: &Arc<Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    use_encryption: bool,
    aead_key: &[u8; 32],
) -> Result<bool> {
    // Extract compose_hash from payload
    if let Some(compose_hash) = plain_msg
        .get("payload")
        .and_then(|p| p.get("compose_hash"))
        .and_then(|v| v.as_str())
    {
        // Count active validators for this compose_hash
        let status_map = validator_status.read().await;
        let mut count = 0;

        for (_hotkey, challenge_statuses) in status_map.iter() {
            if let Some(status) = challenge_statuses.get(compose_hash) {
                if matches!(
                    status.state,
                    platform_api_models::ValidatorChallengeState::Active
                ) {
                    count += 1;
                }
            }
        }

        info!(
            compose_hash = compose_hash,
            validator_count = count,
            "Returning validator count for challenge"
        );

        // Send response
        let response_msg = serde_json::json!({
            "type": "validator_count_result",
            "compose_hash": compose_hash,
            "count": count
        });

        send_message(write_handle, &response_msg, use_encryption, aead_key).await?;
        Ok(true)
    } else {
        warn!("get_validator_count message missing compose_hash in payload");
        Ok(false)
    }
}

/// Handle ORM queries via bridge/proxy
pub async fn handle_orm_query(
    plain_msg: &Value,
    client: &ChallengeWsClient,
    challenge_id: &str,
    orm_gateway: &Arc<tokio::sync::RwLock<platform_api_orm_gateway::SecureORMGateway>>,
    write_handle: &Arc<Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    use_encryption: bool,
    aead_key: &[u8; 32],
) -> Result<bool> {
    info!(
        msg_type = &plain_msg["type"].as_str().unwrap_or(""),
        challenge_id = challenge_id,
        has_orm_gateway = true,
        "Received ORM query message"
    );

    // Extract ORM query from payload
    if let Some(query_json) = plain_msg.get("payload").and_then(|p| p.get("query")) {
        match serde_json::from_value::<platform_api_orm_gateway::ORMQuery>(query_json.clone()) {
            Ok(mut orm_query) => {
                // ALWAYS set schema to challenge schema - platform-api controls schemas
                let schema = if let Some(schema_arc) = &client.schema_name {
                    schema_arc.read().await.clone()
                } else {
                    format!("challenge_{}", challenge_id.replace('-', "_"))
                };
                let schema_for_log = schema.clone();
                orm_query.schema = Some(schema);
                info!(
                    challenge_id = challenge_id,
                    schema = &schema_for_log,
                    "Platform-api set schema for ORM query (overriding any SDK-provided schema)"
                );

                info!(
                    challenge_id = challenge_id,
                    operation = &orm_query.operation,
                    table = &orm_query.table,
                    "Executing ORM query via bridge"
                );

                // Execute query via ORM Gateway
                let orm_gateway_guard = orm_gateway.read().await;
                match orm_gateway_guard.execute_query(orm_query).await {
                    Ok(result) => {
                        // Include query_id if present in request
                        let mut response_msg = serde_json::json!({
                            "type": "orm_result",
                            "result": result
                        });
                        
                        // Extract query_id from multiple possible locations
                        let query_id_opt = plain_msg
                            .get("payload")
                            .and_then(|p| p.get("query_id"))
                            .and_then(|v| v.as_str())
                            .or_else(|| {
                                plain_msg
                                    .get("payload")
                                    .and_then(|p| p.get("query"))
                                    .and_then(|q| q.get("query_id"))
                                    .and_then(|v| v.as_str())
                            })
                            .or_else(|| {
                                plain_msg.get("message_id").and_then(|v| v.as_str())
                            });
                        
                        if let Some(query_id) = query_id_opt {
                            response_msg["query_id"] = serde_json::Value::String(query_id.to_string());
                            response_msg["message_id"] = serde_json::Value::String(query_id.to_string());
                            info!(query_id = query_id, "Including query_id/message_id in ORM result response");
                        } else {
                            warn!("ORM query missing query_id/message_id, response won't be matched");
                        }

                        send_message(write_handle, &response_msg, use_encryption, aead_key).await?;
                        info!(
                            challenge_id = challenge_id,
                            row_count = result.row_count,
                            "ORM query executed successfully via bridge"
                        );
                        Ok(true)
                    }
                    Err(e) => {
                        // Send error response
                        let mut error_msg = serde_json::json!({
                            "type": "error",
                            "error": e.to_string(),
                            "message": e.to_string()
                        });
                        
                        // Extract query_id from multiple possible locations
                        let query_id_opt = plain_msg
                            .get("payload")
                            .and_then(|p| p.get("query_id"))
                            .and_then(|v| v.as_str())
                            .or_else(|| {
                                plain_msg
                                    .get("payload")
                                    .and_then(|p| p.get("query"))
                                    .and_then(|q| q.get("query_id"))
                                    .and_then(|v| v.as_str())
                            })
                            .or_else(|| {
                                plain_msg.get("message_id").and_then(|v| v.as_str())
                            });
                        
                        if let Some(query_id) = query_id_opt {
                            error_msg["query_id"] = serde_json::Value::String(query_id.to_string());
                            error_msg["message_id"] = serde_json::Value::String(query_id.to_string());
                            info!(query_id = query_id, error = %e, "Including query_id/message_id in ORM error response");
                        } else {
                            warn!(error = %e, "ORM error missing query_id/message_id, response won't be matched");
                        }

                        send_message(write_handle, &error_msg, use_encryption, aead_key).await?;
                        warn!(
                            challenge_id = challenge_id,
                            error = %e,
                            "ORM query failed"
                        );
                        Ok(true)
                    }
                }
            }
            Err(e) => {
                warn!("Failed to parse ORM query: {}", e);
                Ok(false)
            }
        }
    } else {
        Ok(false)
    }
}

/// Send a message (encrypted or plain text)
pub async fn send_message(
    write_handle: &Arc<Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
    msg: &Value,
    use_encryption: bool,
    aead_key: &[u8; 32],
) -> Result<()> {
    if !use_encryption {
        // In dev mode without encryption, send plain text message
        let mut write = write_handle.lock().await;
        let msg_str = serde_json::to_string(msg)?;
        write.send(Message::Text(msg_str)).await?;
    } else {
        // Encrypt the message
        let envelope = encrypt_message(msg, aead_key)?;
        let envelope_json = serde_json::json!({
            "enc": envelope.enc,
            "nonce": envelope.nonce,
            "ciphertext": envelope.ciphertext,
        });
        let mut write = write_handle.lock().await;
        write.send(Message::Text(serde_json::to_string(&envelope_json)?)).await?;
    }
    Ok(())
}
