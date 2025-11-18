//! List challenge handlers

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use crate::state::AppState;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use sqlx::Row;
use platform_api_models::{ChallengeListResponse, ChallengeMetadata, ChallengeStatus, ChallengeVisibility, Hotkey, Id};

#[derive(Deserialize)]
pub struct ListChallengesParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// List challenges with pagination
pub async fn list_challenges(
    State(state): State<AppState>,
    Query(params): Query<ListChallengesParams>,
) -> Result<Json<ChallengeListResponse>, StatusCode> {
    tracing::debug!("Starting challenge list query");

    let pool = state.database_pool.as_ref().ok_or_else(|| {
        tracing::error!("Database pool not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    let page = params.page.unwrap_or(1);
    let per_page = params.per_page.unwrap_or(20);
    let offset = (page - 1) * per_page;

    tracing::debug!(
        "Query parameters: page={}, per_page={}, offset={}",
        page, per_page, offset
    );

    // First, get total count
    tracing::debug!("Executing COUNT query");
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM challenges")
        .persistent(false)
        .fetch_one(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to count challenges: {}", e);
            tracing::error!("   Error details: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tracing::debug!("Total challenges in database: {}", total);

    // Query challenges from database with pagination
    #[derive(sqlx::FromRow)]
    struct ChallengeRow {
        id: uuid::Uuid,
        name: String,
        compose_hash: String,
        compose_yaml: String,
        version: String,
        images: Vec<String>,
        resources: JsonValue,
        ports: JsonValue,
        env: JsonValue,
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

    let rows = sqlx::query_as::<_, ChallengeRow>(
        r#"
        SELECT 
            id, name, compose_hash, compose_yaml, version, images,
            resources, ports, env, emission_share, mechanism_id, weight,
            description, mermaid_chart, github_repo, dstack_image,
            created_at, updated_at
        FROM challenges
        ORDER BY created_at DESC
        LIMIT $1 OFFSET $2
        "#,
    )
    .persistent(false)
    .bind(per_page as i64)
    .bind(offset as i64)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query challenges: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let challenges: Vec<ChallengeMetadata> = rows
        .into_iter()
        .map(|row| ChallengeMetadata {
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
        })
        .collect();

    let response = ChallengeListResponse {
        challenges,
        total: total as u64,
        page,
        per_page,
    };

    tracing::debug!(
        "Returning {} challenges (page {}, per_page {}, total {})",
        response.challenges.len(),
        page,
        per_page,
        total
    );

    if response.challenges.is_empty() && total == 0 {
        tracing::warn!("No challenges found in database! Table 'challenges' is empty.");
        tracing::warn!(
            "   Hint: Check if migrations have been applied and if data has been inserted."
        );
    }

    Ok(Json(response))
}

/// Public challenge response structure
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PublicChallengeResponse {
    pub id: String,
    pub name: String,
    pub status: String,
    pub description: Option<String>,
    pub difficulty: Option<String>,
    pub mechanism_id: i16,
    pub emission_share: f64,
    pub github_repo: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub stats: ChallengeStats,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ChallengeStats {
    pub participant_count: usize,
    pub jobs_processed: usize,
    pub success_rate: f64,
    pub pool_size_tao: f64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PublicChallengeListResponse {
    pub challenges: Vec<PublicChallengeResponse>,
    pub total: usize,
}

/// List public challenges (read-only, active challenges only)
pub async fn list_challenges_public(
    State(state): State<AppState>,
) -> Result<Json<PublicChallengeListResponse>, StatusCode> {
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

    // Get active challenges (those with recent jobs)
    let challenges = sqlx::query_as::<_, ChallengeRow>(
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
        // Get stats for this challenge
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

        // Calculate pool size (simplified - would need actual emissions data)
        let pool_size_tao = challenge.emission_share * 1000.0; // Assuming 1000 TAO/day total

        // Determine difficulty
        let difficulty = if challenge.emission_share > 0.5 {
            Some("Expert".to_string())
        } else if challenge.emission_share > 0.3 {
            Some("Hard".to_string())
        } else if challenge.emission_share > 0.1 {
            Some("Medium".to_string())
        } else {
            Some("Easy".to_string())
        };

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
            stats: ChallengeStats {
                participant_count: participant_count as usize,
                jobs_processed: jobs_processed as usize,
                success_rate,
                pool_size_tao,
            },
        });
    }

    Ok(Json(PublicChallengeListResponse {
        total: public_challenges.len(),
        challenges: public_challenges,
    }))
}

