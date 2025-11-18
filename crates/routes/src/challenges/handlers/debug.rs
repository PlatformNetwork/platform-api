use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
};
use serde_json::Value as JsonValue;
use sqlx::Row;
use tracing::debug;

use platform_api::state::AppState;

/// Debug endpoint to diagnose challenges table state
pub async fn debug_challenges(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    debug!("Starting diagnostic query");

    // Get total count
    let total_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM challenges")
        .persistent(false)
        .fetch_one(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to count challenges: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Get table columns and types
    #[derive(sqlx::FromRow)]
    struct ColumnInfo {
        column_name: String,
        data_type: String,
        is_nullable: String,
    }

    let columns: Vec<ColumnInfo> = sqlx::query_as::<_, ColumnInfo>(
        r#"
        SELECT column_name, data_type, is_nullable
        FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'challenges'
        ORDER BY ordinal_position
        "#,
    )
    .persistent(false)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to get column info: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Get first challenge if exists
    let first_challenge: Option<JsonValue> = if total_count > 0 {
        let row = sqlx::query(
            r#"
            SELECT id, name, compose_hash, version, mechanism_id, created_at
            FROM challenges
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .persistent(false)
        .fetch_optional(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to get first challenge: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        row.map(|row| serde_json::json!({
                "id": row.get::<uuid::Uuid, _>("id"),
                "name": row.get::<String, _>("name"),
                "compose_hash": row.get::<String, _>("compose_hash"),
                "version": row.get::<String, _>("version"),
                "mechanism_id": row.get::<i16, _>("mechanism_id"),
                "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            }))
    } else {
        None
    };

    // Check if mechanism_id is SMALLINT or VARCHAR
    let mechanism_id_type: Option<String> = columns
        .iter()
        .find(|c| c.column_name == "mechanism_id")
        .map(|c| c.data_type.clone());

    let response = serde_json::json!({
        "database_connected": true,
        "table_exists": true,
        "total_challenges": total_count,
        "table_columns": columns.iter().map(|c| serde_json::json!({
            "name": c.column_name,
            "type": c.data_type,
            "nullable": c.is_nullable == "YES"
        })).collect::<Vec<_>>(),
        "mechanism_id_type": mechanism_id_type,
        "first_challenge": first_challenge,
        "diagnosis": if total_count == 0 {
            "Table is empty - no challenges found. Check if migrations have been applied and if data has been inserted."
        } else if mechanism_id_type.as_deref() == Some("character varying") {
            "WARNING: mechanism_id is VARCHAR but should be SMALLINT. Migration 004 may not have been applied."
        } else {
            "Table structure looks correct"
        }
    });

    tracing::info!("Diagnostic complete: {} challenges found", total_count);

    Ok(Json(response))
}

