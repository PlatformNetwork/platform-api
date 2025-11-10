use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{
    AttestationRequest, AttestationResponse, AttestationSession, KeyReleaseRequest,
    KeyReleaseResponse, PlatformResult,
};

/// Perform attestation handler
pub async fn attest_handler(
    state: State<AppState>,
    request: Json<AttestationRequest>,
) -> PlatformResult<Json<AttestationResponse>> {
    let response = state.attestation.verify_attestation(request.0).await?;
    Ok(Json(response))
}

/// Get attestation session handler
pub async fn get_attestation_session_handler(
    state: State<AppState>,
    id: Path<Uuid>,
) -> PlatformResult<Json<AttestationSession>> {
    let session = state.attestation.get_session(*id).await?;
    Ok(Json(session))
}

/// Release key handler
pub async fn release_key_handler(
    state: State<AppState>,
    request: Json<KeyReleaseRequest>,
) -> PlatformResult<Json<KeyReleaseResponse>> {
    let response = state.kbs.release_key(request.0).await?;
    Ok(Json(response))
}

/// Verify key handler
pub async fn verify_key_handler(
    state: State<AppState>,
    request: Json<VerifyKeyRequest>,
) -> PlatformResult<Json<platform_api_kbs::VerifyKeyResponse>> {
    let kbs_request = platform_api_kbs::VerifyKeyRequest {
        key_id: request.key_id.clone(),
        session_token: request.session_token.clone(),
        policy: request.policy.clone(),
    };
    let response = state.kbs.verify_key(kbs_request).await?;
    Ok(Json(response))
}

/// List policies handler
pub async fn list_policies_handler(
    state: State<AppState>,
) -> PlatformResult<Json<Vec<platform_api_models::AttestationPolicy>>> {
    let policies = state.attestation.list_policies().await?;
    Ok(Json(policies))
}

/// Get policy handler
pub async fn get_policy_handler(
    state: State<AppState>,
    id: Path<String>,
) -> PlatformResult<Json<platform_api_models::AttestationPolicy>> {
    let policy = state.attestation.get_policy(&id).await?;
    Ok(Json(policy))
}

/// Request to verify a key
#[derive(Debug, serde::Deserialize)]
pub struct VerifyKeyRequest {
    pub key_id: String,
    pub session_token: String,
    pub policy: String,
}

/// Response for key verification
#[derive(Debug, serde::Serialize)]
pub struct VerifyKeyResponse {
    pub is_valid: bool,
    pub key_info: KeyInfo,
    pub error: Option<String>,
}

/// Key information
#[derive(Debug, serde::Serialize)]
pub struct KeyInfo {
    pub key_id: String,
    pub algorithm: String,
    pub key_size: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub usage_count: u32,
    pub max_usage: Option<u32>,
}
