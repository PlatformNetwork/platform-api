//! Active challenges handlers

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
};
use crate::state::AppState;
use serde_json::Value as JsonValue;
use sqlx::Row;

/// Get active challenges only
pub async fn get_active_challenges(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    #[derive(sqlx::FromRow)]
    struct ChallengeRow {
        id: uuid::Uuid,
        name: String,
        compose_hash: String,
        github_repo: Option<String>,
        mechanism_id: i16,
        emission_share: f64,
        resources: JsonValue,
    }

    let rows = sqlx::query_as::<_, ChallengeRow>(
        r#"
        SELECT id, name, compose_hash, github_repo, mechanism_id, emission_share, resources
        FROM challenges
        ORDER BY created_at DESC
        "#,
    )
    .persistent(false)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query active challenges: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::info!(
        "get_active_challenges: returning {} challenges from database",
        rows.len()
    );

    Ok(Json(serde_json::json!({
        "challenges": rows.iter().map(|row| serde_json::json!({
            "id": row.id.to_string(),
            "name": row.name.clone(),
            "status": "Active", // All challenges in database are considered active
            "github_repo": row.github_repo.clone().unwrap_or_default(),
            "github_commit": "", // Not stored in DB, derived from compose_hash
            "resource_requirements": row.resources.clone(),
            "compose_hash": row.compose_hash.clone(),
            "mechanism_id": row.mechanism_id as u8,
            "emission_share": row.emission_share,
        })).collect::<Vec<_>>()
    })))
}

