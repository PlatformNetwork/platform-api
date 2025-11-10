use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use hex;
use platform_api_models::AttestationStatus;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Create artifacts router
pub fn create_router() -> Router<AppState> {
    Router::new().route("/artifacts/:submission_id", get(download_artifact))
}

/// Download encrypted artifact for a submission
/// Requires valid session_token from TEE attestation
pub async fn download_artifact(
    State(state): State<AppState>,
    Path(submission_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response<String>, StatusCode> {
    tracing::info!(
        "Download artifact request for submission: {}",
        submission_id
    );

    // Extract session token from Authorization header
    let auth_header = headers.get(AUTHORIZATION).ok_or_else(|| {
        tracing::error!("Missing Authorization header");
        StatusCode::UNAUTHORIZED
    })?;

    let auth_value = auth_header.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;

    let token = auth_value.strip_prefix("Bearer ").ok_or_else(|| {
        tracing::error!("Invalid Authorization header format");
        StatusCode::BAD_REQUEST
    })?;

    // Decode and verify JWT token
    let claims = state.attestation.verify_token(token).map_err(|e| {
        tracing::error!("Token verification failed: {}", e);
        StatusCode::UNAUTHORIZED
    })?;

    // Extract app_id and instance_id from verified token
    let app_id = claims
        .get("app_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            tracing::error!("Missing app_id in token");
            StatusCode::UNAUTHORIZED
        })?;

    let instance_id = claims
        .get("instance_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            tracing::error!("Missing instance_id in token");
            StatusCode::UNAUTHORIZED
        })?;

    tracing::info!(
        app_id = app_id,
        instance_id = instance_id,
        "TEE verified executor requesting artifact"
    );

    // Fetch encrypted artifact from storage
    let (encrypted_artifact, digest) = state
        .artifact_storage
        .get_artifact(submission_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get artifact: {}", e);
            StatusCode::NOT_FOUND
        })?;

    // Convert to bytes for transmission
    let artifact_bytes = encrypted_artifact.to_bytes();

    // Derive decryption key from session token and submission ID
    let mut key_hasher = sha2::Sha256::new();
    key_hasher.update(token.as_bytes());
    key_hasher.update(submission_id.as_bytes());
    let derived_key = key_hasher.finalize();
    let decryption_key = hex::encode(derived_key);

    // Return encrypted artifact with hash verification
    let response_body = serde_json::json!({
        "artifact": hex::encode(&artifact_bytes),
        "key": decryption_key,
        "digest": digest
    });

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("X-Artifact-Hash", digest)
        .body(serde_json::to_string(&response_body).unwrap())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}
