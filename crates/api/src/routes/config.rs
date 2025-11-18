use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use std::env;
use tracing::warn;
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models::{
    ConfigBackup, ConfigValidationResult, RestoreConfigRequest, TSubnetConfig, UpdateConfigRequest,
    VmComposeResponse, VmHardwareSpec, VmManifestDefaults, VmProvisioningBundle,
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
        .route(
            "/config/compose/validator_vm",
            get(get_validator_vm_compose),
        )
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
) -> Result<Json<VmComposeResponse>, StatusCode> {
    let config = state
        .storage
        .get_vm_compose_config("validator_vm")
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let provisioning = build_validator_provisioning_bundle(&config.vm_type, &config.required_env);

    Ok(Json(VmComposeResponse {
        vm_type: config.vm_type,
        compose_content: config.compose_content,
        description: config.description,
        required_env: config.required_env,
        provisioning,
        updated_at: config.updated_at,
    }))
}

const DEFAULT_ENV_KEYS: &[&str] = &["DSTACK_VMM_URL", "HOTKEY_PASSPHRASE", "VALIDATOR_BASE_URL"];
const DEFAULT_VM_IMAGE: &str = "dstack-0.5.2";
const DEFAULT_VM_VCPU: u32 = 16;
const DEFAULT_VM_MEMORY_MB: u32 = 16 * 1024;
const DEFAULT_VM_DISK_GB: u32 = 200;
const ENV_VALIDATOR_VM_IMAGE: &str = "VALIDATOR_VM_IMAGE";
const ENV_VALIDATOR_VM_VCPU: &str = "VALIDATOR_VM_VCPU";
const ENV_VALIDATOR_VM_MEMORY_MB: &str = "VALIDATOR_VM_MEMORY_MB";
const ENV_VALIDATOR_VM_DISK_GB: &str = "VALIDATOR_VM_DISK_GB";

fn build_validator_provisioning_bundle(
    vm_type: &str,
    required_env: &[String],
) -> VmProvisioningBundle {
    let mut env_keys: Vec<String> = DEFAULT_ENV_KEYS.iter().map(|k| k.to_string()).collect();
    for key in required_env {
        if !env_keys.iter().any(|existing| existing == key) {
            env_keys.push(key.clone());
        }
    }
    // Sort and deduplicate env_keys for consistent hash calculation
    env_keys.sort();
    env_keys.dedup();

    VmProvisioningBundle {
        env_keys,
        manifest_defaults: VmManifestDefaults {
            manifest_version: 2,
            name: Some(vm_type.to_string()),
            runner: "docker-compose".to_string(),
            kms_enabled: true,
            gateway_enabled: true,
            local_key_provider_enabled: false,
            key_provider_id: String::new(),
            public_logs: true,
            public_sysinfo: true,
            public_tcbinfo: true,
            no_instance_id: false,
            secure_time: false,
        },
        vm_parameters: resolve_vm_hardware_spec(vm_type),
    }
}

fn resolve_vm_hardware_spec(vm_type: &str) -> VmHardwareSpec {
    VmHardwareSpec {
        name: Some(vm_type.to_string()),
        image: read_env_string(ENV_VALIDATOR_VM_IMAGE)
            .unwrap_or_else(|| DEFAULT_VM_IMAGE.to_string()),
        vcpu: read_env_u32(ENV_VALIDATOR_VM_VCPU).unwrap_or(DEFAULT_VM_VCPU),
        memory: read_env_u32(ENV_VALIDATOR_VM_MEMORY_MB).unwrap_or(DEFAULT_VM_MEMORY_MB),
        disk_size: read_env_u32(ENV_VALIDATOR_VM_DISK_GB).unwrap_or(DEFAULT_VM_DISK_GB),
        user_config: String::new(),
        ports: Vec::new(),
        hugepages: false,
        pin_numa: false,
        stopped: false,
    }
}

fn read_env_string(key: &str) -> Option<String> {
    match env::var(key) {
        Ok(value) if !value.is_empty() => Some(value),
        Ok(_) => None,
        Err(env::VarError::NotPresent) => None,
        Err(err) => {
            warn!("Failed to read {}: {}", key, err);
            None
        }
    }
}

fn read_env_u32(key: &str) -> Option<u32> {
    let raw = read_env_string(key)?;
    match raw.parse::<u32>() {
        Ok(value) if value > 0 => Some(value),
        Ok(_) => {
            warn!(
                "Ignoring {} override because value must be greater than zero",
                key
            );
            None
        }
        Err(err) => {
            warn!("Failed to parse {} ({}): {}", key, raw, err);
            None
        }
    }
}
