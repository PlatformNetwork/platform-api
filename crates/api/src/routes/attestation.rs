use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use hex::encode as hex_encode;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{
    AttestationRequest, AttestationResponse, AttestationSession, KeyReleaseRequest,
    KeyReleaseResponse,
};

/// Create attestation router
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/attestation/challenge", post(create_challenge))
        .route("/attestation/verify", post(verify_attestation))
        .route("/attest", post(attest))
        .route("/attest/sessions/:id", get(get_attestation_session))
        .route("/keys/release", post(release_key))
        .route("/keys/verify", post(verify_key))
        .route("/policies", get(list_policies))
        .route("/policies/:id", get(get_policy))
}

/// Perform attestation
pub async fn attest(
    State(state): State<AppState>,
    Json(request): Json<AttestationRequest>,
) -> Result<Json<AttestationResponse>, StatusCode> {
    let response = state
        .attestation
        .verify_attestation(request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(response))
}

/// Create an attestation challenge (nonce)
#[derive(Debug, Serialize)]
pub struct ChallengeResponse {
    pub nonce: String,
    pub expires_at: DateTime<Utc>,
}

pub async fn create_challenge(
    _state: State<AppState>,
) -> Result<Json<ChallengeResponse>, StatusCode> {
    let rng = SystemRandom::new();
    let mut nonce = [0u8; 32];
    rng.fill(&mut nonce)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let expires_at = Utc::now() + Duration::seconds(300);
    Ok(Json(ChallengeResponse {
        nonce: hex_encode(nonce),
        expires_at,
    }))
}

/// Verify attestation (alias to /attest for clearer semantics)
pub async fn verify_attestation(
    State(state): State<AppState>,
    Json(request): Json<AttestationRequest>,
) -> Result<Json<AttestationResponse>, StatusCode> {
    let response = state
        .attestation
        .verify_attestation(request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(response))
}

/// Get attestation session
pub async fn get_attestation_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AttestationSession>, StatusCode> {
    let session = state
        .attestation
        .get_session(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(session))
}

/// Release key for attested session
pub async fn release_key(
    State(state): State<AppState>,
    Json(request): Json<KeyReleaseRequest>,
) -> Result<Json<KeyReleaseResponse>, StatusCode> {
    let response = state
        .kbs
        .release_key(request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(response))
}

/// Verify key release
pub async fn verify_key(
    State(state): State<AppState>,
    Json(request): Json<VerifyKeyRequest>,
) -> Result<Json<platform_api_kbs::VerifyKeyResponse>, StatusCode> {
    let kbs_request = platform_api_kbs::VerifyKeyRequest {
        key_id: request.key_id.clone(),
        session_token: request.session_token.clone(),
        policy: request.policy.clone(),
    };
    let response = state
        .kbs
        .verify_key(kbs_request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(response))
}

/// List attestation policies
pub async fn list_policies(
    State(state): State<AppState>,
) -> Result<Json<Vec<platform_api_models::AttestationPolicy>>, StatusCode> {
    let policies = state
        .attestation
        .list_policies()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(policies))
}

/// Get specific policy
pub async fn get_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<platform_api_models::AttestationPolicy>, StatusCode> {
    let policy = state
        .attestation
        .get_policy(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

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
