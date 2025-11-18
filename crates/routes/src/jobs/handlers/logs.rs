use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use platform_api::state::AppState;

use crate::jobs::types::LogStreamParams;

/// Stream test logs in real-time
pub async fn stream_logs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<LogStreamParams>,
) -> Result<Json<JsonValue>, StatusCode> {
    let job_id = id.to_string();

    if let Some(redis) = &state.redis_client {
        let mut logs = Vec::new();

        let mut conn = redis
            .client
            .get_tokio_connection_manager()
            .await
            .map_err(|e| {
                tracing::error!("Failed to connect to Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        // Try to read from Redis Streams first
        let stream_key = format!("job:logs:{}", job_id);
        let stream_result: Result<
            Vec<(String, std::collections::HashMap<String, String>)>,
            redis::RedisError,
        > = redis::cmd("XRANGE")
            .arg(&stream_key)
            .arg("-")
            .arg("+")
            .arg("COUNT")
            .arg(params.limit.unwrap_or(1000))
            .query_async(&mut conn)
            .await;

        match stream_result {
            Ok(stream_entries) => {
                // Convert stream entries to log format
                for (_id, fields) in stream_entries {
                    let log_entry = serde_json::json!({
                        "job_id": fields.get("job_id").cloned(),
                        "file": fields.get("file").cloned(),
                        "line": fields.get("line").cloned(),
                        "line_number": fields.get("line_number").and_then(|v| v.parse::<i32>().ok()),
                        "timestamp": fields.get("timestamp").and_then(|v| v.parse::<f64>().ok()),
                    });
                    logs.push(log_entry);
                }
            }
            Err(e) => {
                tracing::debug!(
                    "Failed to read from Redis Streams, falling back to progress data: {}",
                    e
                );

                // Fallback to reading from progress data
                let key = format!("job:{}:progress", job_id);
                let json_str: Option<String> = redis::cmd("GET")
                    .arg(&key)
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to get job progress from Redis: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?;

                if let Some(json) = json_str {
                    let progress: JsonValue =
                        serde_json::from_str(&json).unwrap_or_else(|_| JsonValue::Null);

                    // Get live_logs if available
                    if let Some(live_logs) = progress.get("live_logs").and_then(|v| v.as_array()) {
                        logs.extend(live_logs.iter().cloned());
                    }

                    // Get current test logs if available
                    if let Some(current_test) = progress.get("current_test") {
                        if let Some(test_logs) = current_test.get("logs").and_then(|v| v.as_array())
                        {
                            logs.extend(test_logs.iter().cloned());
                        }
                    }
                } else {
                    return Err(StatusCode::NOT_FOUND);
                }
            }
        }

        // Apply filters
        let filtered_logs: Vec<JsonValue> = logs
            .into_iter()
            .filter(|log| {
                if let Some(file_filter) = &params.file {
                    if let Some(file) = log.get("file").and_then(|v| v.as_str()) {
                        if !file.contains(file_filter) {
                            return false;
                        }
                    }
                }
                true
            })
            .skip(params.offset.unwrap_or(0))
            .take(params.limit.unwrap_or(1000))
            .collect();

        Ok(Json(serde_json::json!({
            "job_id": job_id,
            "logs": filtered_logs,
            "total": filtered_logs.len(),
            "has_more": filtered_logs.len() == params.limit.unwrap_or(1000)
        })))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

