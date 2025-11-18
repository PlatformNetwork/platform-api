//! Challenge specs handlers

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
};
use crate::state::AppState;

/// Get full challenge specs (for validator polling)
pub async fn get_challenge_specs(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
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

