use super::{Digest, Hotkey, Id};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Challenge visibility level
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChallengeVisibility {
    Public,
    Private,
}

/// Challenge status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChallengeStatus {
    Draft,
    Active,
    Paused,
    Archived,
}

/// Challenge metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeMetadata {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub version: String,
    pub visibility: ChallengeVisibility,
    pub status: ChallengeStatus,
    pub owner: Hotkey,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

/// Harness configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    pub runtime: RuntimeType,
    pub resources: ResourceLimits,
    pub timeout: u64,
    pub environment: BTreeMap<String, String>,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            runtime: RuntimeType::Docker,
            resources: ResourceLimits {
                cpu_cores: 1,
                memory_mb: 1024,
                disk_mb: 10240,
                network_enabled: true,
            },
            timeout: 3600,
            environment: BTreeMap::new(),
        }
    }
}

/// Runtime type for execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuntimeType {
    Standard,
    Docker,
    Sgx,
    Sev,
    WasmEnclave,
}

impl From<&str> for RuntimeType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "docker" => RuntimeType::Docker,
            "sgx" => RuntimeType::Sgx,
            "sev" => RuntimeType::Sev,
            "wasmenclave" | "wasm_enclave" => RuntimeType::WasmEnclave,
            _ => RuntimeType::Standard,
        }
    }
}

impl std::fmt::Display for RuntimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeType::Standard => write!(f, "standard"),
            RuntimeType::Docker => write!(f, "docker"),
            RuntimeType::Sgx => write!(f, "sgx"),
            RuntimeType::Sev => write!(f, "sev"),
            RuntimeType::WasmEnclave => write!(f, "wasmenclave"),
        }
    }
}

/// Resource limits for execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub cpu_cores: u32,
    pub memory_mb: u64,
    pub disk_mb: u64,
    pub network_enabled: bool,
}

/// Challenge creation request
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateChallengeRequest {
    pub name: String,
    pub description: String,
    pub visibility: ChallengeVisibility,
    pub github_repo: Option<String>,
    pub harness_config: HarnessConfig,
    pub dataset_urls: Vec<String>,
}

/// Challenge update request
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateChallengeRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<ChallengeStatus>,
    pub harness_config: Option<HarnessConfig>,
}

/// Challenge list response
#[derive(Debug, Serialize, Deserialize)]
pub struct ChallengeListResponse {
    pub challenges: Vec<ChallengeMetadata>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

/// Challenge detail response
#[derive(Debug, Serialize, Deserialize)]
pub struct ChallengeDetailResponse {
    pub metadata: ChallengeMetadata,
    pub emissions: Option<super::emissions::EmissionSchedule>,
}

/// Emissions schedule for challenges (deprecated, use emissions::EmissionSchedule)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeEmissionsSchedule {
    pub challenge_id: Id,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub emission_rate: f64,
    pub total_emission: Option<f64>,
    pub distribution_curve: ChallengeDistributionCurve,
}

/// Distribution curve for emissions (deprecated, use emissions::DistributionCurve)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChallengeDistributionCurve {
    Linear,
    Exponential {
        decay_factor: f64,
    },
    Step {
        intervals: Vec<(DateTime<Utc>, f64)>,
    },
}

use std::collections::BTreeMap;
use uuid::Uuid;

/// Challenge specification for orchestrating dstack CVMs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeSpec {
    pub id: Uuid,
    pub name: String,
    pub compose_hash: String,
    pub compose_yaml: String,
    pub version: String,
    pub images: Vec<String>, // Array of image@sha256
    pub resources: ChallengeResources,
    pub ports: Vec<ChallengePort>,
    pub env: BTreeMap<String, String>,
    pub emission_share: f64, // 0.0 to 1.0, share of emission for this challenge
    pub mechanism_id: u8,    // Mechanism ID this challenge belongs to (REQUIRED, 0-255)
    pub weight: Option<f64>, // Weight for emission calculation (optional, auto-calculated if None)
    pub description: Option<String>, // Challenge description
    pub mermaid_chart: Option<String>, // Mermaid format chart
    pub github_repo: Option<String>, // GitHub repository URL
    pub dstack_image: Option<String>, // Dstack base image version (e.g., "dstack-0.5.2", default: "dstack-dev-0.5.3")
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Resources for challenge CVM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResources {
    pub vcpu: u32,
    pub memory: String,       // e.g., "4G"
    pub disk: Option<String>, // e.g., "50G"
}

/// Port configuration for challenge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengePort {
    pub container: u16,
    pub protocol: String, // "tcp" or "udp"
}

/// Validator challenge status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum ValidatorChallengeState {
    Active,
    Inactive,
    Failed,
    Provisioning,
    Created,   // Challenge created but not yet active
    Probing,   // Challenge is being probed for health
    Recycling, // Challenge is being recycled
}

/// Validator challenge status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorChallengeStatus {
    pub validator_hotkey: String,
    pub compose_hash: String,
    pub state: ValidatorChallengeState,
    pub last_heartbeat: DateTime<Utc>,
    pub penalty_reason: Option<String>,
}

/// Penalty reason enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PenaltyReason {
    NotServing,
    ProbeTimeout,
    HealthFail,
    NoCallback,
    SLABreach,
}

/// Challenge result for scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResult {
    pub run_id: Uuid,
    pub validator_hotkey: String,
    pub compose_hash: String,
    pub artifact_id: String,
    pub score: f64,
    pub weight: f64,
    pub justification: String,
    pub created_at: DateTime<Utc>,
}

/// Emissions record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeEmission {
    pub block_height: u64,
    pub compose_hash: String,
    pub emission_share: f64,
    pub owner_hotkey: String,
    pub created_at: DateTime<Utc>,
}
