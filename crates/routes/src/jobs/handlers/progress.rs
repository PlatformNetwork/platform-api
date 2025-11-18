use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde_json::Value as JsonValue;
use sqlx::Row;
use uuid::Uuid;

use platform_api::state::AppState;

use crate::jobs::types::TestResultsParams;

/// Get real-time job progress from Redis
pub async fn get_job_progress(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JsonValue>, StatusCode> {
    let job_id = id.to_string();

    if let Some(redis) = &state.redis_client {
        let mut conn = redis
            .client
            .get_tokio_connection_manager()
            .await
            .map_err(|e| {
                tracing::error!("Failed to connect to Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

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
            Ok(Json(progress))
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Get detailed test results from PostgreSQL
pub async fn get_job_test_results(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<TestResultsParams>,
) -> Result<Json<JsonValue>, StatusCode> {
    if let Some(pool) = &state.database_pool {
        let limit = params.limit.unwrap_or(1000);

        let rows = sqlx::query(
            r#"
            SELECT id, job_id, challenge_id, task_id, test_name, status,
                   is_resolved, error_message, execution_time_ms,
                   output_text, logs, metrics, created_at
            FROM job_test_results
            WHERE job_id = $1
            ORDER BY created_at ASC
            LIMIT $2
            "#,
        )
        .bind(id)
        .bind(limit as i64)
        .fetch_all(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to query test results: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        let results: Vec<JsonValue> = rows
            .into_iter()
            .map(|row| {
                serde_json::json!({
                    "id": row.get::<Uuid, _>("id"),
                    "job_id": row.get::<Uuid, _>("job_id"),
                    "challenge_id": row.get::<Uuid, _>("challenge_id"),
                    "task_id": row.get::<String, _>("task_id"),
                    "test_name": row.get::<Option<String>, _>("test_name"),
                    "status": row.get::<String, _>("status"),
                    "is_resolved": row.get::<bool, _>("is_resolved"),
                    "error_message": row.get::<Option<String>, _>("error_message"),
                    "execution_time_ms": row.get::<Option<i64>, _>("execution_time_ms"),
                    "output_text": row.get::<Option<String>, _>("output_text"),
                    "logs": row.get::<JsonValue, _>("logs"),
                    "metrics": row.get::<JsonValue, _>("metrics"),
                    "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                })
            })
            .collect();

        Ok(Json(serde_json::json!({
            "job_id": id,
            "test_results": results,
            "total": results.len(),
        })))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Get currently executing test details
pub async fn get_current_test(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JsonValue>, StatusCode> {
    let job_id = id.to_string();

    if let Some(redis) = &state.redis_client {
        let mut conn = redis
            .client
            .get_tokio_connection_manager()
            .await
            .map_err(|e| {
                tracing::error!("Failed to connect to Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

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

            if let Some(current_test) = progress.get("current_test") {
                Ok(Json(current_test.clone()))
            } else {
                Ok(Json(serde_json::json!({
                    "job_id": job_id,
                    "status": "no_active_test",
                    "message": "No test is currently running"
                })))
            }
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Get resource usage data
pub async fn get_resource_usage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JsonValue>, StatusCode> {
    let job_id = id.to_string();

    if let Some(redis) = &state.redis_client {
        let mut conn = redis
            .client
            .get_tokio_connection_manager()
            .await
            .map_err(|e| {
                tracing::error!("Failed to connect to Redis: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

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

            let mut resource_data = serde_json::json!({
                "job_id": job_id,
                "current_test_resources": null,
                "test_history": []
            });

            // Get current test resource usage
            if let Some(current_test) = progress.get("current_test") {
                if let Some(resource_usage) = current_test.get("resource_usage") {
                    resource_data["current_test_resources"] = resource_usage.clone();
                }
            }

            // Get historical resource usage from test results
            if let Some(results) = progress
                .get("results")
                .and_then(|r| r.get("results"))
                .and_then(|r| r.as_array())
            {
                let history: Vec<JsonValue> = results
                    .iter()
                    .filter_map(|result| {
                        if let (Some(task_id), Some(metrics)) =
                            (result.get("task_id"), result.get("metrics"))
                        {
                            Some(serde_json::json!({
                                "task_id": task_id,
                                "metrics": metrics
                            }))
                        } else {
                            None
                        }
                    })
                    .collect();

                resource_data["test_history"] = serde_json::Value::Array(history);
            }

            Ok(Json(resource_data))
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

