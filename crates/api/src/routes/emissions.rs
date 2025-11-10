use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post, put},
    Router,
};
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{
    CalculateEmissionRequest, CalculateEmissionResponse, ChallengeEmissionMetrics,
    CreateEmissionScheduleRequest, DistributeEmissionRequest, EmissionAggregate, EmissionReport,
    EmissionSchedule, MinerEmissionMetrics, UpdateEmissionScheduleRequest,
    ValidatorEmissionMetrics,
};

/// Create emissions router
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route(
            "/emissions",
            get(list_emissions).post(create_emission_schedule),
        )
        .route(
            "/emissions/:id",
            get(get_emission_schedule).put(update_emission_schedule),
        )
        .route("/emissions/:id/distribute", post(distribute_emission))
        .route("/emissions/calculate", post(calculate_emission))
        .route("/emissions/aggregate", get(get_emission_aggregate))
        .route(
            "/emissions/challenges/:id/metrics",
            get(get_challenge_emission_metrics),
        )
        .route(
            "/emissions/validators/:hotkey/metrics",
            get(get_validator_emission_metrics),
        )
        .route(
            "/emissions/miners/:hotkey/metrics",
            get(get_miner_emission_metrics),
        )
        .route("/emissions/report", get(get_emission_report))
}

/// List emission schedules
pub async fn list_emissions(
    State(state): State<AppState>,
    Query(params): Query<ListEmissionsParams>,
) -> Result<Json<Vec<EmissionSchedule>>, StatusCode> {
    let emissions = state
        .storage
        .list_emission_schedules(params.status, params.emission_type, params.challenge_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(emissions))
}

/// Get emission schedule
pub async fn get_emission_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EmissionSchedule>, StatusCode> {
    let schedule = state
        .storage
        .get_emission_schedule(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(schedule))
}

/// Create emission schedule
pub async fn create_emission_schedule(
    State(state): State<AppState>,
    Json(request): Json<CreateEmissionScheduleRequest>,
) -> Result<Json<EmissionSchedule>, StatusCode> {
    let schedule = state
        .storage
        .create_emission_schedule(request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(schedule))
}

/// Update emission schedule
pub async fn update_emission_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateEmissionScheduleRequest>,
) -> Result<Json<EmissionSchedule>, StatusCode> {
    let schedule = state
        .storage
        .update_emission_schedule(id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(schedule))
}

/// Distribute emission
pub async fn distribute_emission(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<DistributeEmissionRequest>,
) -> Result<StatusCode, StatusCode> {
    state
        .storage
        .distribute_emission(id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Calculate emission
pub async fn calculate_emission(
    State(state): State<AppState>,
    Json(request): Json<CalculateEmissionRequest>,
) -> Result<Json<CalculateEmissionResponse>, StatusCode> {
    let response = state
        .storage
        .calculate_emission(request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(response))
}

/// Get emission aggregate
pub async fn get_emission_aggregate(
    State(state): State<AppState>,
    Query(params): Query<GetEmissionAggregateParams>,
) -> Result<Json<EmissionAggregate>, StatusCode> {
    let aggregate = state
        .storage
        .get_emission_aggregate(params.period_start, params.period_end)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(aggregate))
}

/// Get challenge emission metrics
pub async fn get_challenge_emission_metrics(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ChallengeEmissionMetrics>, StatusCode> {
    let metrics = state
        .storage
        .get_challenge_emission_metrics(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(metrics))
}

/// Get validator emission metrics
pub async fn get_validator_emission_metrics(
    State(state): State<AppState>,
    Path(hotkey): Path<String>,
) -> Result<Json<ValidatorEmissionMetrics>, StatusCode> {
    let metrics = state
        .storage
        .get_validator_emission_metrics(&hotkey)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(metrics))
}

/// Get miner emission metrics
pub async fn get_miner_emission_metrics(
    State(state): State<AppState>,
    Path(hotkey): Path<String>,
) -> Result<Json<MinerEmissionMetrics>, StatusCode> {
    let metrics = state
        .storage
        .get_miner_emission_metrics(&hotkey)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(metrics))
}

/// Get emission report
pub async fn get_emission_report(
    State(state): State<AppState>,
    Query(params): Query<GetEmissionReportParams>,
) -> Result<Json<EmissionReport>, StatusCode> {
    let report = state
        .storage
        .get_emission_report(params.period_start, params.period_end)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(report))
}

/// Query parameters for listing emissions
#[derive(Debug, serde::Deserialize)]
pub struct ListEmissionsParams {
    pub status: Option<String>,
    pub emission_type: Option<String>,
    pub challenge_id: Option<Uuid>,
}

/// Query parameters for getting emission aggregate
#[derive(Debug, serde::Deserialize)]
pub struct GetEmissionAggregateParams {
    pub period_start: chrono::DateTime<chrono::Utc>,
    pub period_end: chrono::DateTime<chrono::Utc>,
}

/// Query parameters for getting emission report
#[derive(Debug, serde::Deserialize)]
pub struct GetEmissionReportParams {
    pub period_start: chrono::DateTime<chrono::Utc>,
    pub period_end: chrono::DateTime<chrono::Utc>,
}
