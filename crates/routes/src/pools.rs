use platform_api::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post, put},
    Router,
};
use platform_api_models::*;
use uuid::Uuid;

pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/validators/:hotkey/pools", post(create_pool))
        .route("/validators/:hotkey/pools", get(list_pools))
        .route("/pools/:id", get(get_pool))
        .route("/pools/:id", put(update_pool))
        .route("/pools/:id", delete(delete_pool))
        .route("/pools/:id/capacity", get(get_pool_capacity))
}

pub async fn create_pool(
    State(state): State<AppState>,
    Path(hotkey): Path<String>,
    Json(request): Json<CreatePoolRequest>,
) -> Result<Json<Pool>, StatusCode> {
    let pool = state
        .storage
        .create_pool(&hotkey, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(pool))
}

pub async fn list_pools(
    State(state): State<AppState>,
    Path(hotkey): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<PoolListResponse>, StatusCode> {
    let page = params.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let per_page = params
        .get("per_page")
        .and_then(|p| p.parse().ok())
        .unwrap_or(20);

    let response = state
        .storage
        .list_pools(Some(&hotkey), page, per_page)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(response))
}

pub async fn get_pool(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Pool>, StatusCode> {
    let pool = state
        .storage
        .get_pool(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(pool))
}

pub async fn update_pool(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdatePoolRequest>,
) -> Result<Json<Pool>, StatusCode> {
    let pool = state
        .storage
        .update_pool(id, request)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(pool))
}

pub async fn delete_pool(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    state
        .storage
        .delete_pool(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_pool_capacity(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PoolCapacitySummary>, StatusCode> {
    let capacity = state
        .storage
        .get_pool_capacity(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(capacity))
}
