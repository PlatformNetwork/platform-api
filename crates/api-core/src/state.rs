// Minimal state module for api-core
// Most of the state implementation is in platform-api crate to avoid circular dependencies

use platform_api_attestation::AttestationService;
use platform_api_builder::BuilderService;
use platform_api_kbs::KeyBrokerService;
use platform_api_scheduler::SchedulerService;
use platform_api_storage::StorageBackend;
use std::sync::Arc;

// Re-export config types
pub use platform_api_attestation::AttestationConfig;
pub use platform_api_builder::BuilderConfig;
pub use platform_api_kbs::KbsConfig;
pub use platform_api_scheduler::SchedulerConfig;
pub use platform_api_storage::StorageConfig;

/// Application configuration
#[derive(Clone)]
pub struct AppConfig {
    pub server_port: u16,
    pub server_host: String,
    pub database_url: String,
    pub storage_config: StorageConfig,
    pub attestation_config: AttestationConfig,
    pub kbs_config: KbsConfig,
    pub scheduler_config: SchedulerConfig,
    pub builder_config: BuilderConfig,
    pub metrics_config: MetricsConfig,
}

/// Metrics configuration
#[derive(Clone)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub port: u16,
    pub path: String,
    pub collect_interval: u64,
}

/// Metrics service
#[derive(Clone)]
pub struct MetricsService {
    pub metrics: String,
}

impl MetricsService {
    pub fn new(_config: &MetricsConfig) -> anyhow::Result<Self> {
        Ok(Self {
            metrics: "# platform-api metrics\n".to_string(),
        })
    }

    pub fn get_metrics(&self) -> anyhow::Result<String> {
        Ok(self.metrics.clone())
    }
}

// Minimal AppState stub - the real implementation is in platform-api crate
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn StorageBackend>,
    pub attestation: Arc<AttestationService>,
    pub kbs: Arc<KeyBrokerService>,
    pub scheduler: Arc<SchedulerService>,
    pub builder: Arc<BuilderService>,
    pub metrics: Arc<MetricsService>,
    pub config: Arc<AppConfig>,
}
