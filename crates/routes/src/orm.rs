use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::post,
    Router,
};
use serde_json::Value;
use tracing::{error, info, warn};

use platform_api_orm_gateway::ORMQuery;
use platform_api::state::AppState;

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
/// DEPRECATED: Use WebSocket for ORM queries instead of HTTP
#[deprecated(
    note = "Use WebSocket connection for ORM queries - HTTP route kept for backward compatibility"
)]
async fn execute_orm_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(query): Json<ORMQuery>,
) -> Result<Json<Value>, StatusCode> {
    // Get validator hotkey from header
    let validator_hotkey = extract_validator_hotkey(&headers).ok_or(StatusCode::UNAUTHORIZED)?;

    warn!(
        validator_hotkey = &validator_hotkey,
        "DEPRECATED: ORM query via HTTP - please use WebSocket connection instead"
    );

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
    let query_with_schema = query;
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
/// DEPRECATED: Use WebSocket for ORM queries instead of HTTP
#[deprecated(
    note = "Use WebSocket connection for ORM queries - HTTP route kept for backward compatibility"
)]
async fn execute_orm_query_with_challenge(
    State(state): State<AppState>,
    Path(challenge_id): Path<String>,
    headers: HeaderMap,
    Json(mut query): Json<ORMQuery>,
) -> Result<Json<Value>, StatusCode> {
    // Get validator hotkey from header
    let validator_hotkey = extract_validator_hotkey(&headers).ok_or(StatusCode::UNAUTHORIZED)?;

    warn!(
        validator_hotkey = &validator_hotkey,
        challenge_id = &challenge_id,
        "DEPRECATED: ORM query via HTTP - please use WebSocket connection instead"
    );

    // Resolve schema: {challenge_name}_v{db_version}
    if query.schema.is_none() {
        // Get challenge_name from database using challenge_id
        let challenge_name = if let Some(pool) = &state.database_pool {
            // Try to parse challenge_id as UUID first
            let challenge_uuid = if let Ok(uuid) = uuid::Uuid::parse_str(&challenge_id) {
                uuid
            } else {
                // If not a UUID, treat it as challenge name directly
                // Normalize challenge name (replace hyphens with underscores for schema)
                let schema_name = challenge_id.replace('-', "_");
                let db_version = query.db_version.unwrap_or(1);
                query.schema = Some(format!("{}_v{}", schema_name, db_version));
                info!(
                    challenge_id = &challenge_id,
                    schema = &query.schema.as_ref().unwrap(),
                    "Resolved schema from challenge_id (non-UUID) and db_version"
                );
                // Use read-only ORM gateway
                let gateway = state
                    .orm_gateway
                    .as_ref()
                    .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
                let gateway = gateway.read().await;
                let result = gateway.execute_read_query(query).await.map_err(|e| {
                    error!("Failed to execute ORM query: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
                return Ok(Json(serde_json::to_value(result).map_err(|e| {
                    error!("Failed to serialize ORM result: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?));
            };

            // Query database for challenge name
            let name_result: Option<String> =
                sqlx::query_scalar("SELECT name FROM challenges WHERE id = $1 LIMIT 1")
                    .bind(challenge_uuid)
                    .fetch_optional(pool.as_ref())
                    .await
                    .map_err(|e| {
                        error!("Failed to query challenge name from database: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?;

            match name_result {
                Some(name) => name,
                None => {
                    error!(
                        challenge_id = &challenge_id,
                        "Challenge not found in database"
                    );
                    return Err(StatusCode::NOT_FOUND);
                }
            }
        } else {
            error!(challenge_id = &challenge_id, "Database pool not available");
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        };

        // Get db_version from query, default to 1 if not provided
        let db_version = query.db_version.unwrap_or(1);

        // Normalize challenge name (replace hyphens with underscores for schema)
        let normalized_name = challenge_name.replace('-', "_");

        // Construct schema: {challenge_name}_v{db_version}
        query.schema = Some(format!("{}_v{}", normalized_name, db_version));

        info!(
            challenge_id = &challenge_id,
            challenge_name = &challenge_name,
            db_version = db_version,
            schema = &query.schema.as_ref().unwrap(),
            "Resolved schema from challenge_name and db_version"
        );
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
