//! CRUD operations for challenges

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use crate::state::AppState;
use uuid::Uuid;
use platform_api_models::{ChallengeMetadata, CreateChallengeRequest, UpdateChallengeRequest};

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

