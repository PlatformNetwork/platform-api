use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post, put},
    Router,
};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use sqlx::Row;
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{
    ChallengeDetailResponse, ChallengeListResponse, ChallengeMetadata, ChallengeStatus,
    ChallengeVisibility, CreateChallengeRequest, Hotkey, Id, PlatformResult,
    UpdateChallengeRequest,
};

/// Create challenges router
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/challenges", get(list_challenges).post(create_challenge))
        .route("/challenges/active", get(get_active_challenges))
        .route("/challenges/debug", get(debug_challenges))
        .route(
            "/challenges/:id",
            get(get_challenge)
                .put(update_challenge)
                .delete(delete_challenge),
        )
        .route("/challenges/:id/emissions", get(get_challenge_emissions))
        .route("/challenges/:id/jobs", get(get_challenge_jobs))
        .route("/challenges/:compose_hash/env-vars", post(store_challenge_env_vars))
}

/// List challenges with pagination
pub async fn list_challenges(
    State(state): State<AppState>,
    Query(params): Query<ListChallengesParams>,
) -> Result<Json<ChallengeListResponse>, StatusCode> {
    eprintln!("🚨 [list_challenges] FUNCTION CALLED - This should appear in logs!");
    tracing::info!("🔍 [list_challenges] Starting challenge list query");

    let pool = state.database_pool.as_ref().ok_or_else(|| {
        tracing::error!("❌ [list_challenges] Database pool not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    let page = params.page.unwrap_or(1);
    let per_page = params.per_page.unwrap_or(20);
    let offset = (page - 1) * per_page;

    tracing::info!(
        "📊 [list_challenges] Query parameters: page={}, per_page={}, offset={}",
        page,
        per_page,
        offset
    );

    // First, get total count
    tracing::info!("🔍 [list_challenges] Executing COUNT query: SELECT COUNT(*) FROM challenges");
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM challenges")
        .persistent(false)
        .fetch_one(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("❌ [list_challenges] Failed to count challenges: {}", e);
            tracing::error!("   Error details: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tracing::info!(
        "📊 [list_challenges] Total challenges in database: {}",
        total
    );

    // Query challenges from database with pagination
    #[derive(sqlx::FromRow)]
    struct ChallengeRow {
        id: uuid::Uuid,
        name: String,
        compose_hash: String,
        compose_yaml: String,
        version: String,
        images: Vec<String>,
        resources: JsonValue,
        ports: JsonValue,
        env: JsonValue,
        emission_share: f64,
        mechanism_id: i16,
        weight: Option<f64>,
        description: Option<String>,
        mermaid_chart: Option<String>,
        github_repo: Option<String>,
        dstack_image: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    let query_sql = r#"
        SELECT 
            id, name, compose_hash, compose_yaml, version, images,
            resources, ports, env, emission_share, mechanism_id, weight,
            description, mermaid_chart, github_repo, dstack_image, 
            created_at, updated_at
        FROM challenges
        ORDER BY created_at DESC
        LIMIT $1 OFFSET $2
        "#;

    tracing::info!(
        "🔍 [list_challenges] Executing SELECT query with LIMIT={}, OFFSET={}",
        per_page,
        offset
    );
    tracing::debug!("   SQL: {}", query_sql);

    let rows = sqlx::query_as::<_, ChallengeRow>(query_sql)
        .persistent(false)
        .bind(per_page as i64)
        .bind(offset as i64)
        .fetch_all(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("❌ [list_challenges] Failed to query challenges: {}", e);
            tracing::error!("   Error details: {:?}", e);
            tracing::error!("   SQL query: {}", query_sql);
            tracing::error!("   Parameters: LIMIT={}, OFFSET={}", per_page, offset);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tracing::info!(
        "📊 [list_challenges] Query returned {} rows from database",
        rows.len()
    );

    if rows.is_empty() && total > 0 {
        tracing::warn!("⚠️  [list_challenges] No rows returned but total count is {} - possible pagination issue", total);
    }

    // Log each challenge found
    for (idx, row) in rows.iter().enumerate() {
        tracing::info!(
            "   Challenge {}: id={}, name={}, compose_hash={}, version={}",
            idx + 1,
            row.id,
            row.name,
            row.compose_hash,
            row.version
        );
    }

    // Convert ChallengeRow to ChallengeMetadata
    let challenges: Vec<ChallengeMetadata> = rows
        .into_iter()
        .map(|row| ChallengeMetadata {
            id: Id::from(row.id),
            name: row.name.clone(),
            description: row.description.unwrap_or_default(),
            version: row.version.clone(),
            visibility: ChallengeVisibility::Public, // Default to Public
            status: ChallengeStatus::Active, // All challenges in database are considered active
            owner: Hotkey::from("platform"), // Default owner
            created_at: row.created_at,
            updated_at: row.updated_at,
            tags: vec![], // No tags for now
        })
        .collect();

    tracing::info!(
        "📊 [list_challenges] Converted {} rows to ChallengeMetadata",
        challenges.len()
    );

    // Apply status filter if provided (in-memory filter since status is not in DB yet)
    let filtered_challenges: Vec<ChallengeMetadata> = if let Some(status_filter) = &params.status {
        tracing::info!(
            "🔍 [list_challenges] Applying status filter: {}",
            status_filter
        );
        let filtered: Vec<ChallengeMetadata> = challenges
            .into_iter()
            .filter(|c| match status_filter.as_str() {
                "Active" => matches!(c.status, ChallengeStatus::Active),
                "Draft" => matches!(c.status, ChallengeStatus::Draft),
                "Paused" => matches!(c.status, ChallengeStatus::Paused),
                "Archived" => matches!(c.status, ChallengeStatus::Archived),
                _ => true,
            })
            .collect();
        tracing::info!(
            "📊 [list_challenges] After status filter: {} challenges",
            filtered.len()
        );
        filtered
    } else {
        challenges
    };

    let response = ChallengeListResponse {
        challenges: filtered_challenges.clone(),
        total: total as u64,
        page,
        per_page,
    };

    tracing::info!(
        "✅ [list_challenges] Returning {} challenges (page {}, per_page {}, total {})",
        response.challenges.len(),
        page,
        per_page,
        total
    );

    if response.challenges.is_empty() && total == 0 {
        tracing::warn!(
            "⚠️  [list_challenges] No challenges found in database! Table 'challenges' is empty."
        );
        tracing::warn!(
            "   Hint: Check if migrations have been applied and if data has been inserted."
        );
    }

    Ok(Json(response))
}

/// Get challenge details
pub async fn get_challenge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ChallengeDetailResponse>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    #[derive(sqlx::FromRow)]
    struct ChallengeRow {
        id: uuid::Uuid,
        name: String,
        compose_hash: String,
        compose_yaml: String,
        version: String,
        images: Vec<String>,
        resources: JsonValue,
        ports: JsonValue,
        env: JsonValue,
        emission_share: f64,
        mechanism_id: i16,
        weight: Option<f64>,
        description: Option<String>,
        mermaid_chart: Option<String>,
        github_repo: Option<String>,
        dstack_image: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    let row = sqlx::query_as::<_, ChallengeRow>(
        r#"
        SELECT 
            id, name, compose_hash, compose_yaml, version, images,
            resources, ports, env, emission_share, mechanism_id, weight,
            description, mermaid_chart, github_repo, dstack_image,
            created_at, updated_at
        FROM challenges
        WHERE id = $1
        "#,
    )
    .persistent(false)
    .bind(id)
    .fetch_optional(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query challenge: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if let Some(row) = row {
        let metadata = ChallengeMetadata {
            id: Id::from(row.id),
            name: row.name,
            description: row.description.unwrap_or_default(),
            version: row.version,
            visibility: ChallengeVisibility::Public,
            status: ChallengeStatus::Active,
            owner: Hotkey::from("platform"),
            created_at: row.created_at,
            updated_at: row.updated_at,
            tags: vec![],
        };

        let response = ChallengeDetailResponse {
            metadata,
            harness: None,
            datasets: vec![],
            emissions: None,
        };

        Ok(Json(response))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// Create new challenge
pub async fn create_challenge(
    State(state): State<AppState>,
    Json(request): Json<CreateChallengeRequest>,
) -> Result<Json<ChallengeMetadata>, StatusCode> {
    let challenge = state
        .builder
        .create_challenge(request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(challenge))
}

/// Update challenge
pub async fn update_challenge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateChallengeRequest>,
) -> Result<Json<ChallengeMetadata>, StatusCode> {
    let challenge = state
        .builder
        .update_challenge(id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(challenge))
}

/// Delete challenge
pub async fn delete_challenge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    state
        .builder
        .delete_challenge(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

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

/// Get active challenges only
pub async fn get_active_challenges(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    #[derive(sqlx::FromRow)]
    struct ChallengeRow {
        id: uuid::Uuid,
        name: String,
        compose_hash: String,
    }

    let rows = sqlx::query_as::<_, ChallengeRow>(
        r#"
        SELECT id, name, compose_hash
        FROM challenges
        ORDER BY created_at DESC
        "#,
    )
    .persistent(false)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query active challenges: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::info!(
        "📋 get_active_challenges: returning {} challenges from database",
        rows.len()
    );

    Ok(Json(serde_json::json!({
        "challenges": rows.iter().map(|row| serde_json::json!({
            "id": row.id,
            "name": row.name,
            "compose_hash": row.compose_hash,
            "status": "Active", // All challenges in database are considered active
        })).collect::<Vec<_>>()
    })))
}

/// Debug endpoint to diagnose challenges table state
pub async fn debug_challenges(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    tracing::info!("🔍 [debug_challenges] Starting diagnostic query");

    // Get total count
    let total_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM challenges")
        .persistent(false)
        .fetch_one(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to count challenges: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Get table columns and types
    #[derive(sqlx::FromRow)]
    struct ColumnInfo {
        column_name: String,
        data_type: String,
        is_nullable: String,
    }

    let columns: Vec<ColumnInfo> = sqlx::query_as::<_, ColumnInfo>(
        r#"
        SELECT column_name, data_type, is_nullable
        FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'challenges'
        ORDER BY ordinal_position
        "#,
    )
    .persistent(false)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to get column info: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Get first challenge if exists
    let first_challenge: Option<serde_json::Value> = if total_count > 0 {
        let row = sqlx::query(
            r#"
            SELECT id, name, compose_hash, version, mechanism_id, created_at
            FROM challenges
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .persistent(false)
        .fetch_optional(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to get first challenge: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        if let Some(row) = row {
            Some(serde_json::json!({
                "id": row.get::<uuid::Uuid, _>("id"),
                "name": row.get::<String, _>("name"),
                "compose_hash": row.get::<String, _>("compose_hash"),
                "version": row.get::<String, _>("version"),
                "mechanism_id": row.get::<i16, _>("mechanism_id"),
                "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            }))
        } else {
            None
        }
    } else {
        None
    };

    // Check if mechanism_id is SMALLINT or VARCHAR
    let mechanism_id_type: Option<String> = columns
        .iter()
        .find(|c| c.column_name == "mechanism_id")
        .map(|c| c.data_type.clone());

    let response = serde_json::json!({
        "database_connected": true,
        "table_exists": true,
        "total_challenges": total_count,
        "table_columns": columns.iter().map(|c| serde_json::json!({
            "name": c.column_name,
            "type": c.data_type,
            "nullable": c.is_nullable == "YES"
        })).collect::<Vec<_>>(),
        "mechanism_id_type": mechanism_id_type,
        "first_challenge": first_challenge,
        "diagnosis": if total_count == 0 {
            "Table is empty - no challenges found. Check if migrations have been applied and if data has been inserted."
        } else if mechanism_id_type.as_deref() == Some("character varying") {
            "WARNING: mechanism_id is VARCHAR but should be SMALLINT. Migration 004 may not have been applied."
        } else {
            "Table structure looks correct"
        }
    });

    tracing::info!(
        "✅ [debug_challenges] Diagnostic complete: {} challenges found",
        total_count
    );

    Ok(Json(response))
}

/// Query parameters for listing challenges
#[derive(Debug, Deserialize)]
pub struct ListChallengesParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub status: Option<String>,
    pub visibility: Option<String>,
}

/// Get all jobs for a challenge with results
pub async fn get_challenge_jobs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<ChallengeJobsParams>,
) -> Result<Json<JsonValue>, StatusCode> {
    // Get jobs for this challenge using scheduler (which uses PostgreSQL)
    let page = params.page.unwrap_or(1);
    let per_page = params.per_page.unwrap_or(20);

    let jobs = state
        .scheduler
        .list_jobs(page, per_page, params.status, Some(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // For each job, get test results count
    let jobs_with_results: Vec<JsonValue> = jobs
        .jobs
        .iter()
        .map(|job| {
            // Count test results for this job (we'll fetch details separately if needed)
            serde_json::json!({
                "job": job,
                "has_test_results": false, // Will be populated if test_results detail is requested
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "challenge_id": id,
        "jobs": jobs_with_results,
        "total": jobs.total,
        "page": jobs.page,
        "per_page": jobs.per_page,
    })))
}

/// Query parameters for challenge jobs
#[derive(Debug, Deserialize)]
pub struct ChallengeJobsParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub status: Option<String>,
    pub include_test_results: Option<bool>,
}

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
        if let Err(e) = state.store_challenge_env_var(&compose_hash, key, value).await {
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
