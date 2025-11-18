use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use platform_api::state::AppState;

use crate::challenges::types::{
    ChallengeStats, PublicChallengeListResponse, PublicChallengeResponse, PublicChallengeRow,
};

/// List public challenges (read-only, active challenges only)
pub async fn list_challenges_public(
    State(state): State<AppState>,
) -> Result<Json<PublicChallengeListResponse>, StatusCode> {
    let pool = state.database_pool.as_ref().ok_or_else(|| {
        tracing::error!("Database pool not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Get active challenges (those with recent jobs)
    let challenges = sqlx::query_as::<_, PublicChallengeRow>(
        r#"
        SELECT DISTINCT
            c.id, c.name, c.mechanism_id, c.emission_share, c.description, c.github_repo,
            c.created_at, c.updated_at
        FROM challenges c
        INNER JOIN jobs j ON j.challenge_id = c.id
        WHERE j.created_at > NOW() - INTERVAL '30 days'
        ORDER BY c.created_at DESC
        "#,
    )
    .persistent(false)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query public challenges: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut public_challenges = Vec::new();

    for challenge in challenges {
        let stats = calculate_challenge_stats(pool.clone(), challenge.id).await?;
        let difficulty = calculate_difficulty(challenge.emission_share);

        public_challenges.push(PublicChallengeResponse {
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
            stats,
        });
    }

    Ok(Json(PublicChallengeListResponse {
        total: public_challenges.len(),
        challenges: public_challenges,
    }))
}

/// Get public challenge details (read-only)
pub async fn get_challenge_public(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PublicChallengeResponse>, StatusCode> {
    let pool = state.database_pool.as_ref().ok_or_else(|| {
        tracing::error!("Database pool not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    let challenge = sqlx::query_as::<_, PublicChallengeRow>(
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

    let stats = calculate_challenge_stats(pool.clone(), challenge.id).await?;
    let difficulty = calculate_difficulty(challenge.emission_share);

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
        stats,
    }))
}

/// Helper function to calculate challenge statistics
async fn calculate_challenge_stats(
    pool: std::sync::Arc<sqlx::PgPool>,
    challenge_id: Uuid,
) -> Result<ChallengeStats, StatusCode> {
    let participant_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(DISTINCT miner_hotkey) 
        FROM jobs 
        WHERE challenge_id = $1
        "#,
    )
    .bind(challenge_id)
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
    .bind(challenge_id)
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
    .bind(challenge_id)
    .persistent(false)
    .fetch_one(pool.as_ref())
    .await
    .unwrap_or(0);

    let success_rate = if jobs_processed > 0 {
        (completed_jobs as f64 / jobs_processed as f64) * 100.0
    } else {
        0.0
    };

    // Calculate pool size (simplified - would need actual emissions data)
    let pool_size_tao = 0.0; // Placeholder, would need actual calculation

    Ok(ChallengeStats {
        participant_count: participant_count as usize,
        jobs_processed: jobs_processed as usize,
        success_rate,
        pool_size_tao,
    })
}

/// Helper function to calculate difficulty based on emission share
fn calculate_difficulty(emission_share: f64) -> Option<String> {
    if emission_share > 0.5 {
        Some("Expert".to_string())
    } else if emission_share > 0.3 {
        Some("Hard".to_string())
    } else if emission_share > 0.1 {
        Some("Medium".to_string())
    } else {
        Some("Easy".to_string())
    }
}

