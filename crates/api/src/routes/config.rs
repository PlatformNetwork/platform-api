use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post, put},
    Router,
};
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{
    ConfigBackup, ConfigResponse, ConfigValidationResult, RestoreConfigRequest, TSubnetConfig,
    UpdateConfigRequest,
};

/// Create config router
pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/subnet/config", get(get_config).put(update_config))
        .route("/subnet/config/validate", post(validate_config))
        .route("/subnet/config/backup", post(create_backup))
        .route("/subnet/config/restore", post(restore_config))
        .route("/subnet/config/backups", get(list_backups))
        .route("/subnet/config/backups/:id", get(get_backup))
        .route("/subnet/config/history", get(get_config_history))
        .route("/config/compose/validator_vm", get(get_validator_vm_compose))
}

/// Get subnet configuration
pub async fn get_config(
    State(state): State<AppState>,
) -> Result<Json<platform_api_models::ConfigResponse>, StatusCode> {
    let config = state
        .storage
        .get_subnet_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = platform_api_models::ConfigResponse {
        config: config.clone(),
        chain_info: platform_api_models::ChainInfo {
            chain_id: "0".to_string(),
            block_number: 0,
            block_hash: "0x0".to_string(),
            timestamp: chrono::Utc::now(),
            validator_count: 0,
            total_stake: 0.0,
            emission_rate: 0.0,
        },
        network_status: platform_api_models::NetworkStatus {
            is_synced: true,
            peer_count: 0,
            last_finalized_block: 0,
            network_latency: 0.0,
            health_score: 1.0,
        },
    };

    Ok(Json(response))
}

/// Update subnet configuration
pub async fn update_config(
    State(state): State<AppState>,
    Json(request): Json<UpdateConfigRequest>,
) -> Result<Json<TSubnetConfig>, StatusCode> {
    let current = state
        .storage
        .get_subnet_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let config = platform_api_models::SubnetConfig {
        owner_hotkey: request.owner_hotkey.clone().unwrap_or(current.owner_hotkey),
        rake: request.rake.unwrap_or(current.rake),
        validator_set_hints: request
            .validator_set_hints
            .clone()
            .unwrap_or(current.validator_set_hints),
        timing_windows: request
            .timing_windows
            .clone()
            .unwrap_or(current.timing_windows),
        emission_schedule: request
            .emission_schedule
            .clone()
            .unwrap_or(current.emission_schedule),
        updated_at: chrono::Utc::now(),
        version: current.version + 1,
    };

    let updated = state
        .storage
        .update_subnet_config(config.clone())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(updated))
}

/// Validate configuration
pub async fn validate_config(
    State(state): State<AppState>,
    Json(request): Json<UpdateConfigRequest>,
) -> Result<Json<ConfigValidationResult>, StatusCode> {
    let result = state
        .storage
        .validate_config(&request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(result))
}

/// Create configuration backup
pub async fn create_backup(
    State(state): State<AppState>,
    Json(request): Json<CreateBackupRequest>,
) -> Result<Json<ConfigBackup>, StatusCode> {
    let backup = state
        .storage
        .create_config_backup(platform_api_storage::CreateBackupRequest {
            reason: request.reason.clone(),
            tags: request.tags.clone(),
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(backup))
}

/// Restore configuration from backup
pub async fn restore_config(
    State(state): State<AppState>,
    Json(request): Json<RestoreConfigRequest>,
) -> Result<StatusCode, StatusCode> {
    state
        .storage
        .restore_config(request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// List configuration backups
pub async fn list_backups(
    State(state): State<AppState>,
) -> Result<Json<Vec<ConfigBackup>>, StatusCode> {
    let backups = state
        .storage
        .list_config_backups()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(backups))
}

/// Get specific backup
pub async fn get_backup(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ConfigBackup>, StatusCode> {
    let backup = state
        .storage
        .get_config_backup(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(backup))
}

/// Get configuration change history
pub async fn get_config_history(
    State(state): State<AppState>,
) -> Result<Json<Vec<platform_api_models::ConfigChangeLog>>, StatusCode> {
    let history = state
        .storage
        .get_config_history()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(history))
}

/// Request to create a backup
#[derive(Debug, serde::Deserialize)]
pub struct CreateBackupRequest {
    pub reason: String,
    pub tags: Option<Vec<String>>,
}

/// Get validator VM compose configuration
pub async fn get_validator_vm_compose(
    State(state): State<AppState>,
) -> Result<Json<platform_api_models::VmComposeResponse>, StatusCode> {
    let config = state
        .storage
        .get_vm_compose_config("validator_vm")
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(platform_api_models::VmComposeResponse {
        vm_type: config.vm_type,
        compose_content: config.compose_content,
        description: config.description,
        updated_at: config.updated_at,
    }))
}
