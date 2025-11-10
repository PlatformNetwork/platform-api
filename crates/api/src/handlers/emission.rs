use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{
    CalculateEmissionRequest, CalculateEmissionResponse, ChallengeEmissionMetrics,
    CreateEmissionScheduleRequest, DistributeEmissionRequest, EmissionAggregate, EmissionReport,
    EmissionSchedule, MinerEmissionMetrics, PlatformResult, UpdateEmissionScheduleRequest,
    ValidatorEmissionMetrics,
};

/// List emission schedules handler
pub async fn list_emissions_handler(
    state: State<AppState>,
    params: Query<ListEmissionsParams>,
) -> PlatformResult<Json<Vec<EmissionSchedule>>> {
    let emissions = state
        .storage
        .list_emission_schedules(
            params.status.clone(),
            params.emission_type.clone(),
            params.challenge_id,
        )
        .await?;

    Ok(Json(emissions))
}

/// Get emission schedule handler
pub async fn get_emission_schedule_handler(
    state: State<AppState>,
    id: Path<Uuid>,
) -> PlatformResult<Json<EmissionSchedule>> {
    let schedule = state.storage.get_emission_schedule(*id).await?;
    Ok(Json(schedule))
}

/// Create emission schedule handler
pub async fn create_emission_schedule_handler(
    state: State<AppState>,
    request: Json<CreateEmissionScheduleRequest>,
) -> PlatformResult<Json<EmissionSchedule>> {
    let schedule = state.storage.create_emission_schedule(request.0).await?;
    Ok(Json(schedule))
}

/// Update emission schedule handler
pub async fn update_emission_schedule_handler(
    state: State<AppState>,
    id: Path<Uuid>,
    request: Json<UpdateEmissionScheduleRequest>,
) -> PlatformResult<Json<EmissionSchedule>> {
    let schedule = state
        .storage
        .update_emission_schedule(*id, request.0)
        .await?;
    Ok(Json(schedule))
}

/// Distribute emission handler
pub async fn distribute_emission_handler(
    state: State<AppState>,
    id: Path<Uuid>,
    request: Json<DistributeEmissionRequest>,
) -> PlatformResult<StatusCode> {
    state.storage.distribute_emission(*id, request.0).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Calculate emission handler
pub async fn calculate_emission_handler(
    state: State<AppState>,
    request: Json<CalculateEmissionRequest>,
) -> PlatformResult<Json<CalculateEmissionResponse>> {
    let response = state.storage.calculate_emission(request.0).await?;
    Ok(Json(response))
}

/// Get emission aggregate handler
pub async fn get_emission_aggregate_handler(
    state: State<AppState>,
    params: Query<GetEmissionAggregateParams>,
) -> PlatformResult<Json<EmissionAggregate>> {
    let aggregate = state
        .storage
        .get_emission_aggregate(params.period_start, params.period_end)
        .await?;

    Ok(Json(aggregate))
}

/// Get challenge emission metrics handler
pub async fn get_challenge_emission_metrics_handler(
    state: State<AppState>,
    id: Path<Uuid>,
) -> PlatformResult<Json<ChallengeEmissionMetrics>> {
    let metrics = state.storage.get_challenge_emission_metrics(*id).await?;
    Ok(Json(metrics))
}

/// Get validator emission metrics handler
pub async fn get_validator_emission_metrics_handler(
    state: State<AppState>,
    hotkey: Path<String>,
) -> PlatformResult<Json<ValidatorEmissionMetrics>> {
    let metrics = state
        .storage
        .get_validator_emission_metrics(&hotkey)
        .await?;
    Ok(Json(metrics))
}

/// Get miner emission metrics handler
pub async fn get_miner_emission_metrics_handler(
    state: State<AppState>,
    hotkey: Path<String>,
) -> PlatformResult<Json<MinerEmissionMetrics>> {
    let metrics = state.storage.get_miner_emission_metrics(&hotkey).await?;
    Ok(Json(metrics))
}

/// Get emission report handler
pub async fn get_emission_report_handler(
    state: State<AppState>,
    params: Query<GetEmissionReportParams>,
) -> PlatformResult<Json<EmissionReport>> {
    let report = state
        .storage
        .get_emission_report(params.period_start, params.period_end)
        .await?;

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
