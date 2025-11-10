use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{
    ConfigBackup, ConfigResponse, ConfigValidationResult, PlatformResult, RestoreConfigRequest,
    SubnetConfig, UpdateConfigRequest,
};

/// Get subnet configuration handler
pub async fn get_config_handler(state: State<AppState>) -> PlatformResult<Json<ConfigResponse>> {
    let config = state.storage.get_subnet_config().await?;
    Ok(Json(ConfigResponse {
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
    }))
}

/// Update subnet configuration handler
pub async fn update_config_handler(
    state: State<AppState>,
    request: Json<UpdateConfigRequest>,
) -> PlatformResult<Json<SubnetConfig>> {
    let current = state.storage.get_subnet_config().await.unwrap_or_default();
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
    let updated = state.storage.update_subnet_config(config.clone()).await?;
    Ok(Json(updated))
}

/// Validate configuration handler
pub async fn validate_config_handler(
    state: State<AppState>,
    request: Json<UpdateConfigRequest>,
) -> PlatformResult<Json<ConfigValidationResult>> {
    let result = state.storage.validate_config(&request).await?;
    Ok(Json(result))
}

/// Create configuration backup handler
pub async fn create_backup_handler(
    state: State<AppState>,
    request: Json<CreateBackupRequest>,
) -> PlatformResult<Json<ConfigBackup>> {
    let backup = state
        .storage
        .create_config_backup(platform_api_storage::CreateBackupRequest {
            reason: request.reason.clone(),
            tags: request.tags.clone(),
        })
        .await?;
    Ok(Json(backup))
}

/// Restore configuration handler
pub async fn restore_config_handler(
    state: State<AppState>,
    request: Json<RestoreConfigRequest>,
) -> PlatformResult<StatusCode> {
    state.storage.restore_config(request.0).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List configuration backups handler
pub async fn list_backups_handler(
    state: State<AppState>,
) -> PlatformResult<Json<Vec<ConfigBackup>>> {
    let backups = state.storage.list_config_backups().await?;
    Ok(Json(backups))
}

/// Get specific backup handler
pub async fn get_backup_handler(
    state: State<AppState>,
    id: Path<Uuid>,
) -> PlatformResult<Json<ConfigBackup>> {
    let backup = state.storage.get_config_backup(*id).await?;
    Ok(Json(backup))
}

/// Get configuration change history handler
pub async fn get_config_history_handler(
    state: State<AppState>,
) -> PlatformResult<Json<Vec<platform_api_models::ConfigChangeLog>>> {
    let history = state.storage.get_config_history().await?;
    Ok(Json(history))
}

/// Request to create a backup
#[derive(Debug, serde::Deserialize)]
pub struct CreateBackupRequest {
    pub reason: String,
    pub tags: Option<Vec<String>>,
}
