use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::post,
    Router,
};
use serde_json::Value;
use tracing::{info, warn};

use crate::orm_gateway::ORMQuery;
use crate::state::AppState;

/// Create ORM router for validator routes (read-only)
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/orm/query", post(execute_orm_query))
        .route(
            "/challenges/:challenge_id/orm/query",
            post(execute_orm_query_with_challenge),
        )
}

/// Execute ORM query (read-only for validator)
/// Validator hotkey must be in header X-Validator-Hotkey
async fn execute_orm_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(query): Json<ORMQuery>,
) -> Result<Json<Value>, StatusCode> {
    // Get validator hotkey from header
    let validator_hotkey = extract_validator_hotkey(&headers).ok_or(StatusCode::UNAUTHORIZED)?;

    info!(
        validator_hotkey = &validator_hotkey,
        operation = &query.operation,
        "Validator executing ORM query (read-only)"
    );

    // Use read-only ORM gateway
    let orm_gateway = state
        .orm_gateway_readonly
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Auto-set schema if challenge_id can be extracted from query or validator
    let mut query_with_schema = query;
    // Schema should be provided in the query, or use the challenge-specific endpoint
    // For now, require schema to be provided in query

    // Execute query (read-only gateway will reject write operations)
    let orm_gateway_guard = orm_gateway.read().await;
    match orm_gateway_guard.execute_query(query_with_schema).await {
        Ok(result) => Ok(Json(serde_json::json!({
            "success": true,
            "result": result
        }))),
        Err(e) => {
            warn!(
                validator_hotkey = &validator_hotkey,
                error = %e,
                "ORM query failed"
            );
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

/// Execute ORM query for a specific challenge (read-only for validator)
async fn execute_orm_query_with_challenge(
    State(state): State<AppState>,
    Path(challenge_id): Path<String>,
    headers: HeaderMap,
    Json(mut query): Json<ORMQuery>,
) -> Result<Json<Value>, StatusCode> {
    // Get validator hotkey from header
    let validator_hotkey = extract_validator_hotkey(&headers).ok_or(StatusCode::UNAUTHORIZED)?;

    // Set schema automatically
    if query.schema.is_none() {
        query.schema = Some(format!("challenge_{}", challenge_id.replace('-', "_")));
    }

    info!(
        validator_hotkey = &validator_hotkey,
        challenge_id = &challenge_id,
        operation = &query.operation,
        "Validator executing ORM query for challenge (read-only)"
    );

    // Use read-only ORM gateway
    let orm_gateway = state
        .orm_gateway_readonly
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Execute query (read-only gateway will reject write operations)
    let orm_gateway_guard = orm_gateway.read().await;
    match orm_gateway_guard.execute_query(query).await {
        Ok(result) => Ok(Json(serde_json::json!({
            "success": true,
            "result": result
        }))),
        Err(e) => {
            warn!(
                validator_hotkey = &validator_hotkey,
                challenge_id = &challenge_id,
                error = %e,
                "ORM query failed"
            );
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

/// Extract validator hotkey from header
fn extract_validator_hotkey(header_map: &HeaderMap) -> Option<String> {
    header_map
        .get("X-Validator-Hotkey")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}
