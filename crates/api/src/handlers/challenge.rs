use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{
    ChallengeDetailResponse, ChallengeListResponse, ChallengeMetadata, CreateChallengeRequest,
    PlatformResult, UpdateChallengeRequest,
};

/// List challenges handler
pub async fn list_challenges_handler(
    state: State<AppState>,
    params: Query<ListChallengesParams>,
) -> PlatformResult<Json<ChallengeListResponse>> {
    let challenges = state
        .storage
        .list_challenges(
            params.page.unwrap_or(1),
            params.per_page.unwrap_or(20),
            params.status.clone(),
            params.visibility.clone(),
        )
        .await?;

    Ok(Json(challenges))
}

/// Get challenge handler
pub async fn get_challenge_handler(
    state: State<AppState>,
    id: Path<Uuid>,
) -> PlatformResult<Json<ChallengeDetailResponse>> {
    let challenge = state.storage.get_challenge(*id).await?;
    Ok(Json(challenge))
}

/// Create challenge handler
pub async fn create_challenge_handler(
    state: State<AppState>,
    request: Json<CreateChallengeRequest>,
) -> PlatformResult<Json<ChallengeMetadata>> {
    let challenge = state.builder.create_challenge(request.0).await?;
    Ok(Json(challenge))
}

/// Update challenge handler
pub async fn update_challenge_handler(
    state: State<AppState>,
    id: Path<Uuid>,
    request: Json<UpdateChallengeRequest>,
) -> PlatformResult<Json<ChallengeMetadata>> {
    let challenge = state.builder.update_challenge(*id, request.0).await?;
    Ok(Json(challenge))
}

/// Delete challenge handler
pub async fn delete_challenge_handler(
    state: State<AppState>,
    id: Path<Uuid>,
) -> PlatformResult<StatusCode> {
    state.builder.delete_challenge(*id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get challenge emissions handler
pub async fn get_challenge_emissions_handler(
    state: State<AppState>,
    id: Path<Uuid>,
) -> PlatformResult<Json<platform_api_models::EmissionsSchedule>> {
    let emissions = state.storage.get_challenge_emissions(*id).await?;
    Ok(Json(emissions))
}

/// Query parameters for listing challenges
#[derive(Debug, serde::Deserialize)]
pub struct ListChallengesParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub status: Option<String>,
    pub visibility: Option<String>,
}
