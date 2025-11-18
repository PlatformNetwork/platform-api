//! Challenge environment variables handlers

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use crate::state::AppState;
use serde::Deserialize;

/// Request body for storing challenge environment variables
#[derive(Debug, Deserialize)]
pub struct StoreChallengeEnvVarsRequest {
    pub env_vars: std::collections::HashMap<String, String>,
}

/// Store challenge environment variables (encrypted)
pub async fn store_challenge_env_vars(
    State(state): State<AppState>,
    Path(compose_hash): Path<String>,
    Json(request): Json<StoreChallengeEnvVarsRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    tracing::info!(
        compose_hash = %compose_hash,
        count = request.env_vars.len(),
        "Storing environment variables for challenge"
    );

    // Store each environment variable
    for (key, value) in request.env_vars.iter() {
        if let Err(e) = state
            .store_challenge_env_var(&compose_hash, key, value)
            .await
        {
            tracing::error!(
                compose_hash = %compose_hash,
                key = %key,
                error = %e,
                "Failed to store environment variable"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    tracing::info!(
        compose_hash = %compose_hash,
        count = request.env_vars.len(),
        "Successfully stored environment variables for challenge"
    );

    Ok(Json(serde_json::json!({
        "compose_hash": compose_hash,
        "stored_count": request.env_vars.len(),
        "message": "Environment variables stored successfully"
    })))
}

