//! Types for challenge runner

use serde::{Deserialize, Serialize};

/// Challenge instance running in API mode
#[derive(Debug, Clone)]
pub struct ChallengeInstance {
    pub challenge_id: String,
    pub name: String,
    pub version: String,
    pub compose_hash: String,
    pub cvm_instance_id: Option<String>,
    pub cvm_api_url: Option<String>,
    pub schema_name: String,
    pub db_version: Option<u32>, // Database version from challenge SDK (set_db_version)
    pub is_running: bool,
    pub ws_started: bool, // WebSocket connection to challenge CVM started
}

/// Configuration for challenge runner
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeRunnerConfig {
    pub enabled: bool,
    pub vmm_url: String,
    pub gateway_url: String,
    pub max_concurrent_challenges: usize,
    pub migration_timeout: u64,
    pub cvm_check_interval: u64,
}

impl Default for ChallengeRunnerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            vmm_url: "http://127.0.0.1:11530".to_string(),
            gateway_url: "https://gateway.platform.network".to_string(),
            max_concurrent_challenges: 10,
            migration_timeout: 300, // 5 minutes
            cvm_check_interval: 30, // 30 seconds
        }
    }
}

/// Challenge metadata
#[derive(Debug, Clone)]
pub struct ChallengeMetadata {
    pub name: String,
    pub version: String,
    pub github_repo: String,
    pub compose_hash: String,
    pub dstack_image: Option<String>,
    pub images: Vec<String>,
    pub compose_yaml: String,
}

/// Challenge status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeStatus {
    pub challenge_id: String,
    pub is_running: bool,
    pub is_healthy: bool,
    pub schema_name: String,
    pub version: String,
    pub cvm_instance_id: Option<String>,
}
