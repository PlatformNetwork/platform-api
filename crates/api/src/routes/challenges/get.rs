//! Get challenge handlers

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use crate::state::AppState;
use uuid::Uuid;
use platform_api_models::{
    ChallengeDetailResponse, ChallengeMetadata, ChallengeStatus, ChallengeVisibility, Hotkey, Id,
};

/// Get challenge details
pub async fn get_challenge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ChallengeDetailResponse>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    #[derive(sqlx::FromRow)]
    struct ChallengeRow {
        id: uuid::Uuid,
        name: String,
        compose_hash: String,
        compose_yaml: String,
        version: String,
        images: Vec<String>,
        resources: serde_json::Value,
        ports: serde_json::Value,
        env: serde_json::Value,
        emission_share: f64,
        mechanism_id: i16,
        weight: Option<f64>,
        description: Option<String>,
        mermaid_chart: Option<String>,
        github_repo: Option<String>,
        dstack_image: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    let row = sqlx::query_as::<_, ChallengeRow>(
        r#"
        SELECT 
            id, name, compose_hash, compose_yaml, version, images,
            resources, ports, env, emission_share, mechanism_id, weight,
            description, mermaid_chart, github_repo, dstack_image,
            created_at, updated_at
        FROM challenges
        WHERE id = $1
        "#,
    )
    .persistent(false)
    .bind(id)
    .fetch_optional(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query challenge: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if let Some(row) = row {
        let metadata = ChallengeMetadata {
            id: Id::from(row.id),
            name: row.name,
            description: row.description.unwrap_or_default(),
            version: row.version,
            visibility: ChallengeVisibility::Public,
            status: ChallengeStatus::Active,
            owner: Hotkey::from("platform"),
            created_at: row.created_at,
            updated_at: row.updated_at,
            tags: vec![],
        };

        let response = ChallengeDetailResponse {
            metadata,
            harness: None,
            datasets: vec![],
            emissions: None,
        };

        Ok(Json(response))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

use super::list::{PublicChallengeResponse, ChallengeStats};

/// Get public challenge details (read-only)
pub async fn get_challenge_public(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PublicChallengeResponse>, StatusCode> {
    let pool = state.database_pool.as_ref().ok_or_else(|| {
        tracing::error!("Database pool not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    #[derive(sqlx::FromRow)]
    struct ChallengeRow {
        id: uuid::Uuid,
        name: String,
        mechanism_id: i16,
        emission_share: f64,
        description: Option<String>,
        github_repo: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    let challenge = sqlx::query_as::<_, ChallengeRow>(
        r#"
        SELECT 
            id, name, mechanism_id, emission_share, description, github_repo,
            created_at, updated_at
        FROM challenges
        WHERE id = $1
        "#,
    )
    .bind(id)
    .persistent(false)
    .fetch_optional(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query challenge: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let challenge = challenge.ok_or(StatusCode::NOT_FOUND)?;

    // Get stats
    let participant_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(DISTINCT miner_hotkey) 
        FROM jobs 
        WHERE challenge_id = $1
        "#,
    )
    .bind(challenge.id)
    .persistent(false)
    .fetch_one(pool.as_ref())
    .await
    .unwrap_or(0);

    let jobs_processed: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) 
        FROM jobs 
        WHERE challenge_id = $1
        "#,
    )
    .bind(challenge.id)
    .persistent(false)
    .fetch_one(pool.as_ref())
    .await
    .unwrap_or(0);

    let completed_jobs: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) 
        FROM jobs 
        WHERE challenge_id = $1 AND status = 'completed'
        "#,
    )
    .bind(challenge.id)
    .persistent(false)
    .fetch_one(pool.as_ref())
    .await
    .unwrap_or(0);

    let success_rate = if jobs_processed > 0 {
        (completed_jobs as f64 / jobs_processed as f64) * 100.0
    } else {
        0.0
    };

    let pool_size_tao = challenge.emission_share * 1000.0; // Simplified calculation

    let difficulty = if challenge.emission_share > 0.5 {
        Some("Expert".to_string())
    } else if challenge.emission_share > 0.3 {
        Some("Hard".to_string())
    } else if challenge.emission_share > 0.1 {
        Some("Medium".to_string())
    } else {
        Some("Easy".to_string())
    };

    Ok(Json(PublicChallengeResponse {
        id: challenge.id.to_string(),
        name: challenge.name,
        status: "live".to_string(),
        description: challenge.description,
        difficulty,
        mechanism_id: challenge.mechanism_id,
        emission_share: challenge.emission_share,
        github_repo: challenge.github_repo,
        created_at: challenge.created_at.to_rfc3339(),
        updated_at: challenge.updated_at.to_rfc3339(),
        stats: ChallengeStats {
            participant_count: participant_count as usize,
            jobs_processed: jobs_processed as usize,
            success_rate,
            pool_size_tao,
        },
    }))
}

