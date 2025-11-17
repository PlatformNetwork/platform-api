use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sp_core::{
    crypto::{Pair, Ss58Codec},
    sr25519,
};
use tracing::{error, info, warn};

use crate::routes::metagraph::get_metagraph_cache;
use crate::state::AppState;

/// Serialize JSON with sorted keys to match Python's json.dumps(..., sort_keys=True)
/// This ensures signature verification works correctly between Python client and Rust server
fn serialize_json_canonical(value: &Value) -> Result<String> {
    // Convert Value to sorted Value, then serialize
    let sorted_value = sort_json_keys(value);
    // Use serde_json with compact format (no spaces) to match Python's separators=(",", ":")
    let mut writer = Vec::new();
    {
        let mut ser = serde_json::Serializer::new(&mut writer);
        serde::Serialize::serialize(&sorted_value, &mut ser)?;
    }
    Ok(String::from_utf8(writer)?)
}

/// Recursively sort JSON object keys to match Python's sort_keys=True behavior
fn sort_json_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            // Convert to BTreeMap to sort keys
            let mut sorted_map = Map::new();
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            
            for k in keys {
                if let Some(v) = map.get(&k) {
                    sorted_map.insert(k, sort_json_keys(v));
                }
            }
            Value::Object(sorted_map)
        }
        Value::Array(arr) => {
            let sorted_arr: Vec<Value> = arr.iter().map(sort_json_keys).collect();
            Value::Array(sorted_arr)
        }
        _ => value.clone(),
    }
}

/// Signature verification error
#[derive(Debug)]
pub enum SignatureError {
    MissingHeader(String),
    InvalidTimestamp,
    InvalidSignature,
    InvalidHotkey,
    HotkeyNotInMetagraph,
    ChallengeNotFound,
    CvmUnavailable,
}

impl IntoResponse for SignatureError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            SignatureError::MissingHeader(header) => (
                StatusCode::BAD_REQUEST,
                format!("Missing header: {}", header),
            ),
            SignatureError::InvalidTimestamp => (
                StatusCode::UNAUTHORIZED,
                "Timestamp too old or invalid".to_string(),
            ),
            SignatureError::InvalidSignature => {
                (StatusCode::UNAUTHORIZED, "Invalid signature".to_string())
            }
            SignatureError::InvalidHotkey => {
                (StatusCode::BAD_REQUEST, "Invalid hotkey format".to_string())
            }
            SignatureError::HotkeyNotInMetagraph => (
                StatusCode::FORBIDDEN,
                "Hotkey not found in metagraph".to_string(),
            ),
            SignatureError::ChallengeNotFound => {
                (StatusCode::NOT_FOUND, "Challenge not found".to_string())
            }
            SignatureError::CvmUnavailable => (
                StatusCode::BAD_GATEWAY,
                "Challenge CVM is not available".to_string(),
            ),
        };

        let body = serde_json::json!({
            "error": message
        });
        (status, axum::Json(body)).into_response()
    }
}

/// Verify miner signature for public route requests
async fn verify_miner_signature(
    headers: &HeaderMap,
    body_json: &Value,
) -> Result<String, SignatureError> {
    // Extract required headers
    let signature = headers
        .get("X-Signature")
        .ok_or_else(|| SignatureError::MissingHeader("X-Signature".to_string()))?
        .to_str()
        .map_err(|_| SignatureError::InvalidSignature)?;

    let nonce = headers
        .get("X-Nonce")
        .ok_or_else(|| SignatureError::MissingHeader("X-Nonce".to_string()))?
        .to_str()
        .map_err(|_| SignatureError::InvalidTimestamp)?;

    let timestamp_str = headers
        .get("X-Timestamp")
        .ok_or_else(|| SignatureError::MissingHeader("X-Timestamp".to_string()))?
        .to_str()
        .map_err(|_| SignatureError::InvalidTimestamp)?;

    let miner_hotkey = headers
        .get("X-Miner-Hotkey")
        .ok_or_else(|| SignatureError::MissingHeader("X-Miner-Hotkey".to_string()))?
        .to_str()
        .map_err(|_| SignatureError::InvalidHotkey)?;

    // Verify timestamp is recent (within 30 seconds)
    let timestamp = timestamp_str
        .parse::<u64>()
        .map_err(|_| SignatureError::InvalidTimestamp)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if now.saturating_sub(timestamp) > 30 {
        return Err(SignatureError::InvalidTimestamp);
    }

    // Verify hotkey exists in metagraph cache
    let cache = get_metagraph_cache();
    let cache_guard = cache.read().await;
    if !cache_guard.contains(miner_hotkey) {
        warn!(hotkey = miner_hotkey, "Hotkey not found in metagraph cache");
        return Err(SignatureError::HotkeyNotInMetagraph);
    }

    // Calculate SHA256 hash of JSON body
    // IMPORTANT: Must match Python's json.dumps(body, separators=(",", ":"), sort_keys=True)
    // Rust's serde_json::to_string doesn't sort keys by default, so we need to sort them
    let body_json_str = serialize_json_canonical(body_json)
        .context("Failed to serialize body JSON")
        .map_err(|_| SignatureError::InvalidSignature)?;

    let mut hasher = Sha256::new();
    hasher.update(body_json_str.as_bytes());
    let body_hash = hasher.finalize();
    let body_hash_hex = hex::encode(body_hash);

    // Reconstruct message: nonce + timestamp + body_hash_hex
    let message_str = format!("{}{}{}", nonce, timestamp_str, body_hash_hex);
    let message_bytes = message_str.as_bytes();

    // Decode signature from hex (64 bytes for sr25519)
    let signature_bytes = hex::decode(signature).map_err(|_| SignatureError::InvalidSignature)?;

    if signature_bytes.len() != 64 {
        return Err(SignatureError::InvalidSignature);
    }

    let mut sig_array = [0u8; 64];
    sig_array.copy_from_slice(&signature_bytes);
    let signature = sr25519::Signature::from(sig_array);

    // Decode hotkey from SS58 format to sr25519::Public
    let public_key =
        sr25519::Public::from_ss58check(miner_hotkey).map_err(|_| SignatureError::InvalidHotkey)?;

    // Verify signature using sr25519::Pair::verify()
    let is_valid = sr25519::Pair::verify(&signature, message_bytes, &public_key);

    if !is_valid {
        warn!(hotkey = miner_hotkey, "Signature verification failed");
        return Err(SignatureError::InvalidSignature);
    }

    info!(hotkey = miner_hotkey, "Signature verified successfully");
    Ok(miner_hotkey.to_string())
}

/// Check if an endpoint is a public read-only endpoint that doesn't require signature
fn is_public_readonly_endpoint(route_name: &str) -> bool {
    matches!(route_name, "get_agent_status" | "list_agents" | "list_agent_jobs")
}

/// Proxy GET request to challenge CVM
async fn proxy_get_to_challenge(
    state: &AppState,
    challenge_name: &str,
    route_name: &str,
    query_params: &str,
    verified_hotkey: Option<&str>,
) -> Result<Response, SignatureError> {
    // Get challenge runner from state
    let challenge_runner = state
        .challenge_runner
        .as_ref()
        .ok_or_else(|| SignatureError::ChallengeNotFound)?;

    // Find challenge by name or ID using public method
    let running_challenges = challenge_runner.list_running_challenges().await;

    // Search for challenge by name or challenge_id
    let challenge_instance = running_challenges
        .iter()
        .find(|inst| inst.name == challenge_name || inst.challenge_id == challenge_name);

    if challenge_instance.is_none() {
        warn!(
            challenge_name = challenge_name,
            "Challenge not found or not running"
        );
        return Err(SignatureError::ChallengeNotFound);
    }

    let instance = challenge_instance.unwrap();

    // Get CVM API URL
    let cvm_api_url = instance
        .cvm_api_url
        .as_ref()
        .ok_or_else(|| SignatureError::CvmUnavailable)?;

    // Build target URL: {cvm_api_url}/sdk/public/{route_name}?{query_params}
    let target_url = if query_params.is_empty() {
        format!(
            "{}/sdk/public/{}",
            cvm_api_url.trim_end_matches('/'),
            route_name
        )
    } else {
        format!(
            "{}/sdk/public/{}?{}",
            cvm_api_url.trim_end_matches('/'),
            route_name,
            query_params
        )
    };

    info!(
        challenge_name = challenge_name,
        route_name = route_name,
        target_url = &target_url,
        "Proxying GET request to challenge CVM"
    );

    // Create HTTP client
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true) // Accept self-signed certs from CVMs
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| {
            error!("Failed to create HTTP client: {}", e);
            SignatureError::CvmUnavailable
        })?;

    // Forward GET request to challenge CVM
    let mut request_builder = client.get(&target_url);

    // Add verified hotkey header if available (for signed requests)
    if let Some(hotkey) = verified_hotkey {
        request_builder = request_builder.header("X-Verified-Miner-Hotkey", hotkey);
    }

    // Add CHUTES API token header if available
    if let Some(chutes_token) = state.get_chutes_api_token().await {
        request_builder = request_builder.header("X-CHUTES-API-TOKEN", chutes_token);
    }

    let response = request_builder.send().await.map_err(|e| {
        error!(
            target_url = &target_url,
            error = %e,
            "Failed to proxy GET request to challenge CVM"
        );
        SignatureError::CvmUnavailable
    })?;

    // Get response status and body
    let status = response.status();
    let body = response.text().await.map_err(|e| {
        error!("Failed to read response body: {}", e);
        SignatureError::CvmUnavailable
    })?;

    // Parse JSON body if possible, otherwise return as-is
    let json_body: Value =
        serde_json::from_str(&body).unwrap_or_else(|_| serde_json::json!({ "raw_response": body }));

    // Convert to axum response
    let axum_response = axum::response::Response::builder()
        .status(status.as_u16())
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json_body).unwrap_or_default(),
        ))
        .map_err(|_| SignatureError::CvmUnavailable)?;

    Ok(axum_response)
}

/// Proxy request to challenge CVM
async fn proxy_to_challenge(
    state: &AppState,
    challenge_name: &str,
    route_name: &str,
    body_json: Value,
    verified_hotkey: &str,
) -> Result<Response, SignatureError> {
    // Get challenge runner from state
    let challenge_runner = state
        .challenge_runner
        .as_ref()
        .ok_or_else(|| SignatureError::ChallengeNotFound)?;

    // Find challenge by name or ID using public method
    let running_challenges = challenge_runner.list_running_challenges().await;

    // Search for challenge by name or challenge_id
    let challenge_instance = running_challenges
        .iter()
        .find(|inst| inst.name == challenge_name || inst.challenge_id == challenge_name);

    if challenge_instance.is_none() {
        warn!(
            challenge_name = challenge_name,
            "Challenge not found or not running"
        );
        return Err(SignatureError::ChallengeNotFound);
    }

    let instance = challenge_instance.unwrap();

    // Get CVM API URL
    let cvm_api_url = instance
        .cvm_api_url
        .as_ref()
        .ok_or_else(|| SignatureError::CvmUnavailable)?;

    // Build target URL: {cvm_api_url}/sdk/public/{route_name}
    let target_url = format!(
        "{}/sdk/public/{}",
        cvm_api_url.trim_end_matches('/'),
        route_name
    );

    info!(
        challenge_name = challenge_name,
        route_name = route_name,
        target_url = &target_url,
        "Proxying request to challenge CVM"
    );

    // Create HTTP client
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true) // Accept self-signed certs from CVMs
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| {
            error!("Failed to create HTTP client: {}", e);
            SignatureError::CvmUnavailable
        })?;

    // Forward request to challenge CVM with verified hotkey in header
    // Also include CHUTES API token from platform-api if available
    let mut request_builder = client
        .post(&target_url)
        .header("X-Verified-Miner-Hotkey", verified_hotkey)
        .header("Content-Type", "application/json");

    // Add CHUTES API token header if available (for LLM validation during agent upload)
    if let Some(chutes_token) = state.get_chutes_api_token().await {
        request_builder = request_builder.header("X-CHUTES-API-TOKEN", chutes_token);
        info!("Including CHUTES API token in proxy request for LLM validation");
    } else {
        warn!("CHUTES API token not available - LLM validation may fail during agent upload");
    }

    let response = request_builder.json(&body_json).send().await.map_err(|e| {
        error!(
            target_url = &target_url,
            error = %e,
            "Failed to proxy request to challenge CVM"
        );
        SignatureError::CvmUnavailable
    })?;

    // Get response status and body
    let status = response.status();
    let body = response.text().await.map_err(|e| {
        error!("Failed to read response body: {}", e);
        SignatureError::CvmUnavailable
    })?;

    // Parse JSON body if possible, otherwise return as-is
    let json_body: Value =
        serde_json::from_str(&body).unwrap_or_else(|_| serde_json::json!({ "raw_response": body }));

    // Convert to axum response
    let axum_response = axum::response::Response::builder()
        .status(status.as_u16())
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json_body).unwrap_or_default(),
        ))
        .map_err(|_| SignatureError::CvmUnavailable)?;

    Ok(axum_response)
}

/// Handle challenge public route GET request
async fn handle_challenge_public_route_get(
    State(state): State<AppState>,
    Path((challenge_name, route_name)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, SignatureError> {
    // Extract query string from URI
    let query_string = uri.query().unwrap_or("");

    // Check if this is a public read-only endpoint that doesn't require signature
    let verified_hotkey = if is_public_readonly_endpoint(&route_name) {
        // Public read-only endpoint: no signature required
        info!(
            route_name = &route_name,
            "Public read-only endpoint, skipping signature verification"
        );
        None
    } else {
        // Protected endpoint: verify signature with empty JSON body
        let empty_body = serde_json::json!({});
        let hotkey = verify_miner_signature(&headers, &empty_body).await?;
        Some(hotkey)
    };

    // Proxy GET request to challenge
    proxy_get_to_challenge(
        &state,
        &challenge_name,
        &route_name,
        query_string,
        verified_hotkey.as_deref(),
    )
    .await
}

/// Handle challenge public route POST request
async fn handle_challenge_public_route(
    State(state): State<AppState>,
    Path((challenge_name, route_name)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Result<Response, SignatureError> {
    // Read request body
    let body_bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|_| SignatureError::InvalidSignature)?;

    // Parse JSON body
    let body_json: Value =
        serde_json::from_slice(&body_bytes).map_err(|_| SignatureError::InvalidSignature)?;

    // Verify signature
    let verified_hotkey = verify_miner_signature(&headers, &body_json).await?;

    // Proxy to challenge
    proxy_to_challenge(
        &state,
        &challenge_name,
        &route_name,
        body_json,
        &verified_hotkey,
    )
    .await
}

/// Create challenge proxy router
pub fn create_router() -> Router<AppState> {
    Router::new().route(
        "/api/challenges/:challenge_name/public/:route_name",
        get(handle_challenge_public_route_get).post(handle_challenge_public_route),
    )
}
