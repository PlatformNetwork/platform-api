//! Challenge emissions handlers

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use crate::state::AppState;
use uuid::Uuid;

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

