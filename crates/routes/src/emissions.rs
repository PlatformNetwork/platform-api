use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use uuid::Uuid;

use platform_api::state::AppState;
use platform_api_models::{
    CalculateEmissionRequest, CalculateEmissionResponse, ChallengeEmissionMetrics,
    ChallengeEmissions, CreateEmissionScheduleRequest, DistributeEmissionRequest,
    EmissionAggregate, EmissionReport, EmissionSchedule, MechanismEmissions, MinerEmissionMetrics,
    SubnetEmissions, UpdateEmissionScheduleRequest, ValidatorEmissionMetrics,
};
use tracing::error;

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
        .route("/emissions/subnet/:netuid", get(get_subnet_emissions))
        .route(
            "/emissions/subnet/:netuid/mechanisms",
            get(get_subnet_mechanisms_emissions),
        )
        .route(
            "/emissions/subnet/:netuid/mechanisms/:mechanism_id",
            get(get_mechanism_emissions),
        )
        .route(
            "/emissions/subnet/:netuid/challenges/:challenge_id",
            get(get_challenge_emissions_from_subnet),
        )
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

/// Get subnet emissions from blockchain
pub async fn get_subnet_emissions(
    State(state): State<AppState>,
    Path(netuid): Path<u16>,
) -> Result<Json<SubnetEmissions>, StatusCode> {
    let bittensor = state.bittensor.as_ref().ok_or_else(|| {
        error!("BittensorService not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Get challenge registry
    let challenge_registry = state.challenge_registry.read().await;

    // Calculate emissions
    let emissions = bittensor
        .calculate_subnet_emissions(&challenge_registry)
        .await
        .map_err(|e| {
            error!("Failed to calculate subnet emissions: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Verify netuid matches
    if emissions.netuid != netuid {
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(Json(emissions))
}

/// Get all mechanisms emissions for a subnet
pub async fn get_subnet_mechanisms_emissions(
    State(state): State<AppState>,
    Path(netuid): Path<u16>,
) -> Result<Json<Vec<MechanismEmissions>>, StatusCode> {
    let bittensor = state.bittensor.as_ref().ok_or_else(|| {
        error!("BittensorService not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Get subnet emissions first
    let challenge_registry = state.challenge_registry.read().await;
    let subnet_emissions = bittensor
        .calculate_subnet_emissions(&challenge_registry)
        .await
        .map_err(|e| {
            error!("Failed to calculate subnet emissions: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if subnet_emissions.netuid != netuid {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Convert mechanism breakdowns to MechanismEmissions
    let mechanisms: Vec<MechanismEmissions> = subnet_emissions
        .mechanisms
        .into_iter()
        .map(|m| MechanismEmissions {
            netuid,
            mechanism_id: m.mechanism_id,
            emission_percentage: m.emission_percentage,
            daily_emissions_tao: m.daily_emissions_tao,
            challenges: m.challenges,
        })
        .collect();

    Ok(Json(mechanisms))
}

/// Get emissions for a specific mechanism
pub async fn get_mechanism_emissions(
    State(state): State<AppState>,
    Path((netuid, mechanism_id)): Path<(u16, u8)>,
) -> Result<Json<MechanismEmissions>, StatusCode> {
    let bittensor = state.bittensor.as_ref().ok_or_else(|| {
        error!("BittensorService not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Get challenge registry
    let challenge_registry = state.challenge_registry.read().await;

    // Calculate mechanism emissions
    let emissions = bittensor
        .calculate_mechanism_emissions(mechanism_id, &challenge_registry)
        .await
        .map_err(|e| {
            error!("Failed to calculate mechanism emissions: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Verify netuid matches
    if emissions.netuid != netuid {
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(Json(emissions))
}

/// Get emissions for a specific challenge
pub async fn get_challenge_emissions_from_subnet(
    State(state): State<AppState>,
    Path((netuid, challenge_id)): Path<(u16, Uuid)>,
) -> Result<Json<ChallengeEmissions>, StatusCode> {
    let bittensor = state.bittensor.as_ref().ok_or_else(|| {
        error!("BittensorService not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Get challenge registry
    let challenge_registry = state.challenge_registry.read().await;

    // Calculate challenge emissions
    let emissions = bittensor
        .calculate_challenge_emissions(challenge_id, &challenge_registry)
        .await
        .map_err(|e| {
            error!("Failed to calculate challenge emissions: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Verify netuid matches
    if emissions.netuid != netuid {
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(Json(emissions))
}
