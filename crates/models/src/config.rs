use super::Hotkey;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Subnet configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubnetConfig {
    pub owner_hotkey: Hotkey,
    pub rake: f64,
    pub validator_set_hints: Vec<ValidatorHint>,
    pub timing_windows: TimingWindows,
    pub emission_schedule: EmissionSchedule,
    pub updated_at: DateTime<Utc>,
    pub version: u32,
}

impl Default for SubnetConfig {
    fn default() -> Self {
        Self {
            owner_hotkey: "0x0".to_string(),
            rake: 0.0,
            validator_set_hints: vec![],
            timing_windows: TimingWindows {
                job_claim_window: 300,
                job_execution_timeout: 3600,
                weight_submission_window: 300,
                emission_distribution_window: 86400,
                attestation_timeout: 30,
            },
            emission_schedule: EmissionSchedule {
                total_supply: 1000000.0,
                emission_rate: 1.0,
                distribution_period: 86400,
                owner_rake_rate: 0.1,
                validator_reward_rate: 0.4,
                miner_reward_rate: 0.5,
                start_time: Utc::now(),
                end_time: None,
            },
            updated_at: Utc::now(),
            version: 1,
        }
    }
}

/// Validator hint for subnet configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorHint {
    pub hotkey: Hotkey,
    pub uid: Option<u32>,
    pub stake: Option<f64>,
    pub performance_score: Option<f64>,
    pub last_seen: Option<DateTime<Utc>>,
}

/// Timing windows for operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingWindows {
    pub job_claim_window: u64,
    pub job_execution_timeout: u64,
    pub weight_submission_window: u64,
    pub emission_distribution_window: u64,
    pub attestation_timeout: u64,
}

/// Emission schedule configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionSchedule {
    pub total_supply: f64,
    pub emission_rate: f64,
    pub distribution_period: u64,
    pub owner_rake_rate: f64,
    pub validator_reward_rate: f64,
    pub miner_reward_rate: f64,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
}

/// Configuration update request
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateConfigRequest {
    pub owner_hotkey: Option<Hotkey>,
    pub rake: Option<f64>,
    pub validator_set_hints: Option<Vec<ValidatorHint>>,
    pub timing_windows: Option<TimingWindows>,
    pub emission_schedule: Option<EmissionSchedule>,
}

/// Configuration response
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigResponse {
    pub config: SubnetConfig,
    pub chain_info: ChainInfo,
    pub network_status: NetworkStatus,
}

/// Chain information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainInfo {
    pub chain_id: String,
    pub block_number: u64,
    pub block_hash: String,
    pub timestamp: DateTime<Utc>,
    pub validator_count: u32,
    pub total_stake: f64,
    pub emission_rate: f64,
}

/// Network status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub is_synced: bool,
    pub peer_count: u32,
    pub last_finalized_block: u64,
    pub network_latency: f64,
    pub health_score: f64,
}

/// Configuration validation result
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub suggestions: Vec<String>,
}

/// Configuration change log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigChangeLog {
    pub id: uuid::Uuid,
    pub change_type: ConfigChangeType,
    pub old_value: Option<serde_json::Value>,
    pub new_value: serde_json::Value,
    pub changed_by: Hotkey,
    pub timestamp: DateTime<Utc>,
    pub reason: Option<String>,
}

/// Configuration change type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConfigChangeType {
    OwnerHotkeyChanged,
    RakeUpdated,
    ValidatorSetUpdated,
    TimingWindowsUpdated,
    EmissionScheduleUpdated,
    FullConfigUpdated,
}

/// Configuration backup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBackup {
    pub id: uuid::Uuid,
    pub config: SubnetConfig,
    pub created_at: DateTime<Utc>,
    pub created_by: Hotkey,
    pub version: u32,
    pub checksum: String,
}

/// Configuration restore request
#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreConfigRequest {
    pub backup_id: uuid::Uuid,
    pub reason: String,
    pub confirm: bool,
}

/// Configuration metrics
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigMetrics {
    pub total_changes: u64,
    pub changes_last_24h: u64,
    pub avg_change_frequency: f64,
    pub most_changed_field: String,
    pub config_stability_score: f64,
}
