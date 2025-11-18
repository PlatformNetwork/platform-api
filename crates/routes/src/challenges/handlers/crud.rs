use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use platform_api::state::AppState;
use platform_api_models::{
    ChallengeDetailResponse, ChallengeMetadata, ChallengeStatus, ChallengeVisibility,
    CreateChallengeRequest, Hotkey, Id, UpdateChallengeRequest,
};

use crate::challenges::types::ChallengeRow;

/// Get challenge details
pub async fn get_challenge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ChallengeDetailResponse>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

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

/// Create new challenge
pub async fn create_challenge(
    State(state): State<AppState>,
    Json(request): Json<CreateChallengeRequest>,
) -> Result<Json<ChallengeMetadata>, StatusCode> {
    let challenge = state
        .builder
        .create_challenge(request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(challenge))
}

/// Update challenge
pub async fn update_challenge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateChallengeRequest>,
) -> Result<Json<ChallengeMetadata>, StatusCode> {
    let challenge = state
        .builder
        .update_challenge(id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(challenge))
}

/// Delete challenge
pub async fn delete_challenge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    state
        .builder
        .delete_challenge(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Get challenge emissions
pub async fn get_challenge_emissions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<platform_api_models::EmissionsSchedule>, StatusCode> {
    let emissions = state
        .storage
        .get_challenge_emissions(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(emissions))
}

