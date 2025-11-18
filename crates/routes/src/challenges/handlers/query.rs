use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use serde_json::Value as JsonValue;
use sqlx::Row;

use platform_api::state::AppState;
use platform_api_models::{
    ChallengeListResponse, ChallengeMetadata, ChallengeStatus, ChallengeVisibility, Hotkey, Id,
};
use tracing::debug;

use crate::challenges::types::{ChallengeRow, ListChallengesParams, MinimalChallengeRow};

/// List challenges with pagination
pub async fn list_challenges(
    State(state): State<AppState>,
    Query(params): Query<ListChallengesParams>,
) -> Result<Json<ChallengeListResponse>, StatusCode> {
    debug!("Starting challenge list query");

    let pool = state.database_pool.as_ref().ok_or_else(|| {
        tracing::error!("Database pool not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    let page = params.page.unwrap_or(1);
    let per_page = params.per_page.unwrap_or(20);
    let offset = (page - 1) * per_page;

    debug!(
        "Query parameters: page={}, per_page={}, offset={}",
        page, per_page, offset
    );

    // First, get total count
    debug!("Executing COUNT query");
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM challenges")
        .persistent(false)
        .fetch_one(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to count challenges: {}", e);
            tracing::error!("   Error details: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    debug!("Total challenges in database: {}", total);

    // Query challenges from database with pagination
    let query_sql = r#"
        SELECT 
            id, name, compose_hash, compose_yaml, version, images,
            resources, ports, env, emission_share, mechanism_id, weight,
            description, mermaid_chart, github_repo, dstack_image, 
            created_at, updated_at
        FROM challenges
        ORDER BY created_at DESC
        LIMIT $1 OFFSET $2
        "#;

    debug!(
        "Executing SELECT query with LIMIT={}, OFFSET={}",
        per_page, offset
    );
    tracing::debug!("   SQL: {}", query_sql);

    let rows = sqlx::query_as::<_, ChallengeRow>(query_sql)
        .persistent(false)
        .bind(per_page as i64)
        .bind(offset as i64)
        .fetch_all(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to query challenges: {}", e);
            tracing::error!("   Error details: {:?}", e);
            tracing::error!("   SQL query: {}", query_sql);
            tracing::error!("   Parameters: LIMIT={}, OFFSET={}", per_page, offset);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    debug!("Query returned {} rows from database", rows.len());

    if rows.is_empty() && total > 0 {
        tracing::warn!(
            "No rows returned but total count is {} - possible pagination issue",
            total
        );
    }

    // Log each challenge found
    for (idx, row) in rows.iter().enumerate() {
        tracing::info!(
            "   Challenge {}: id={}, name={}, compose_hash={}, version={}",
            idx + 1,
            row.id,
            row.name,
            row.compose_hash,
            row.version
        );
    }

    // Convert ChallengeRow to ChallengeMetadata
    let challenges: Vec<ChallengeMetadata> = rows
        .into_iter()
        .map(|row| ChallengeMetadata {
            id: Id::from(row.id),
            name: row.name.clone(),
            description: row.description.unwrap_or_default(),
            version: row.version.clone(),
            visibility: ChallengeVisibility::Public, // Default to Public
            status: ChallengeStatus::Active, // All challenges in database are considered active
            owner: Hotkey::from("platform"), // Default owner
            created_at: row.created_at,
            updated_at: row.updated_at,
            tags: vec![], // No tags for now
        })
        .collect();

    debug!("Converted {} rows to ChallengeMetadata", challenges.len());

    // Apply status filter if provided (in-memory filter since status is not in DB yet)
    let filtered_challenges: Vec<ChallengeMetadata> = if let Some(status_filter) = &params.status {
        debug!("Applying status filter: {}", status_filter);
        let filtered: Vec<ChallengeMetadata> = challenges
            .into_iter()
            .filter(|c| match status_filter.as_str() {
                "Active" => matches!(c.status, ChallengeStatus::Active),
                "Draft" => matches!(c.status, ChallengeStatus::Draft),
                "Paused" => matches!(c.status, ChallengeStatus::Paused),
                "Archived" => matches!(c.status, ChallengeStatus::Archived),
                _ => true,
            })
            .collect();
        debug!("After status filter: {} challenges", filtered.len());
        filtered
    } else {
        challenges
    };

    let response = ChallengeListResponse {
        challenges: filtered_challenges.clone(),
        total: total as u64,
        page,
        per_page,
    };

    debug!(
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

/// Get active challenges only
pub async fn get_active_challenges(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let rows = sqlx::query_as::<_, MinimalChallengeRow>(
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

/// Get full challenge specs (for validator polling)
pub async fn get_challenge_specs(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, StatusCode> {
    tracing::info!("get_challenge_specs: retrieving full challenge specifications");

    // Use the same list_challenges method as WebSocket
    let challenges = state.list_challenges().await;

    tracing::info!(
        "get_challenge_specs: returning {} challenge specs",
        challenges.len()
    );

    Ok(Json(serde_json::json!({
        "type": "challenges:list",
        "challenges": challenges
    })))
}

