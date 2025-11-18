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
        .route("/pools/:pool_id/nodes", post(add_node))
        .route("/pools/:pool_id/nodes", get(list_nodes))
        .route("/nodes/:id", get(get_node))
        .route("/nodes/:id", put(update_node))
        .route("/nodes/:id", delete(delete_node))
        .route("/nodes/:id/health", get(get_node_health))
}

pub async fn add_node(
    State(state): State<AppState>,
    Path(pool_id): Path<Uuid>,
    Json(request): Json<AddNodeRequest>,
) -> Result<Json<Node>, StatusCode> {
    let node = state
        .storage
        .add_node(pool_id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(node))
}

pub async fn list_nodes(
    State(state): State<AppState>,
    Path(pool_id): Path<Uuid>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<NodeListResponse>, StatusCode> {
    let page = params.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let per_page = params
        .get("per_page")
        .and_then(|p| p.parse().ok())
        .unwrap_or(20);

    let response = state
        .storage
        .list_nodes(Some(pool_id), page, per_page)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(response))
}

pub async fn get_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Node>, StatusCode> {
    let node = state
        .storage
        .get_node(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(node))
}

pub async fn update_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateNodeRequest>,
) -> Result<Json<Node>, StatusCode> {
    let node = state
        .storage
        .update_node(id, request)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(node))
}

pub async fn delete_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    state
        .storage
        .delete_node(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_node_health(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeHealth>, StatusCode> {
    let node = state
        .storage
        .get_node(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(node.health))
}
