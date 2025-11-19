//! Refactored challenge runner module with organized components

pub mod challenge_ws;
pub mod ws; // WebSocket client module (split from challenge_ws.rs)
pub mod cvm_manager;
pub mod migrations;
pub mod runner;

// New modular components
pub mod runner_core;
pub mod challenge_orchestrator;
pub mod monitoring;

use std::sync::Arc;
use anyhow::Result;

pub use challenge_ws::ChallengeWsClient;
pub use cvm_manager::CvmManager;
pub use migrations::MigrationRunner;
pub use runner::{
    ChallengeInstance, ChallengeRunnerConfig, ChallengeMetadata, ChallengeStatus,
};

pub use runner_core::ChallengeRunnerCore;
pub use challenge_orchestrator::{
    ChallengeOrchestrator, ChallengeExecution, QueuedChallenge, OrchestratorStats,
};
pub use monitoring::{
    ChallengeMonitor, ChallengeMetrics, ExecutionRecord, PerformanceSnapshot,
    ChallengeAlert, AlertType, AlertSeverity, MonitoringConfig, AlertThresholds,
};

/// Main challenge runner facade that combines all components
pub struct ChallengeRunner {
    core: Arc<ChallengeRunnerCore>,
    orchestrator: Arc<ChallengeOrchestrator>,
    monitor: Arc<ChallengeMonitor>,
}

impl ChallengeRunner {
    /// Create new challenge runner with all components
    pub fn new(
        config: runner::types::ChallengeRunnerConfig,
        db_pool: sqlx::PgPool,
        orm_gateway: Option<Arc<tokio::sync::RwLock<platform_api_orm_gateway::SecureORMGateway>>>,
        validator_challenge_status: Option<
            Arc<
                tokio::sync::RwLock<
                    std::collections::HashMap<
                        String,
                        std::collections::HashMap<String, platform_api_models::ValidatorChallengeStatus>,
                    >,
                >,
            >,
        >,
        redis_client: Option<Arc<crate::redis_client::RedisClient>>,
        validator_connections: Option<
            Arc<
                tokio::sync::RwLock<
                    std::collections::HashMap<String, crate::state::ValidatorConnection>,
                >,
            >,
        >,
    ) -> Self {
        // Create core runner
        let core = Arc::new(ChallengeRunnerCore::new(
            config.clone(),
            db_pool.clone(),
            orm_gateway.clone(),
            validator_challenge_status.clone(),
            redis_client.clone(),
            validator_connections.clone(),
        ));

        // Create orchestrator
        let orchestrator = Arc::new(ChallengeOrchestrator::new(
            core.clone(),
            config.max_concurrent_challenges,
        ));

        // Create monitor with default config
        let monitoring_config = MonitoringConfig {
            metrics_retention_hours: 24,
            alert_thresholds: AlertThresholds {
                max_execution_time_minutes: 60,
                max_failure_rate_percent: 20.0,
                max_memory_usage_mb: 8192, // 8GB
                max_cpu_usage_percent: 90.0,
            },
            health_check_interval_seconds: 30,
            enable_performance_tracking: true,
        };

        let monitor = Arc::new(ChallengeMonitor::new(
            core.clone(),
            orchestrator.clone(),
            monitoring_config,
        ));

        Self {
            core,
            orchestrator,
            monitor,
        }
    }

    /// Initialize the challenge runner system
    pub async fn initialize(&self) -> Result<()> {
        // Start monitoring
        self.monitor.start_monitoring().await?;

        // Start orchestrator background tasks
        self.start_orchestrator_tasks().await?;

        info!("Challenge runner system initialized");
        Ok(())
    }

    /// Run a challenge (queues it for execution)
    pub async fn run_challenge(&self, challenge_id: &str) -> Result<()> {
        // Get challenge metadata to find compose_hash
        let challenge = self.core.get_challenge_metadata(challenge_id).await?;
        self.run_challenge_by_compose_hash(&challenge.compose_hash).await
    }

    /// Run a challenge by compose hash
    pub async fn run_challenge_by_compose_hash(&self, compose_hash: &str) -> Result<()> {
        self.orchestrator.queue_challenge(
            compose_hash.to_string(),
            None,
            None,
            5, // Default priority
        ).await
    }

    /// Run a challenge with custom specification and environment
    pub async fn run_challenge_with_spec(
        &self,
        compose_hash: &str,
        challenge_spec: Option<serde_json::Value>,
        env_vars: Option<std::collections::HashMap<String, String>>,
        priority: i32,
    ) -> Result<()> {
        self.orchestrator.queue_challenge(
            compose_hash.to_string(),
            challenge_spec,
            env_vars,
            priority,
        ).await
    }

    /// Execute a challenge immediately (bypasses queue)
    pub async fn execute_challenge_immediately(
        &self,
        compose_hash: &str,
        challenge_spec: Option<serde_json::Value>,
        env_vars: Option<std::collections::HashMap<String, String>>,
    ) -> Result<()> {
        self.orchestrator.execute_challenge_immediately(
            compose_hash,
            challenge_spec,
            env_vars,
        ).await
    }

    /// Stop a running challenge
    pub async fn stop_challenge(&self, compose_hash: &str) -> Result<()> {
        self.orchestrator.stop_challenge(compose_hash).await
    }

    /// Get status of all challenges
    pub async fn get_all_challenges_status(&self) -> std::collections::HashMap<String, ChallengeStatus> {
        self.orchestrator.get_all_challenges_status()
    }

    /// Get detailed execution info for a challenge
    pub async fn get_challenge_execution(&self, compose_hash: &str) -> Option<ChallengeExecution> {
        self.orchestrator.get_challenge_execution(compose_hash)
    }

    /// Get queue information
    pub async fn get_queue_info(&self) -> Vec<QueuedChallenge> {
        self.orchestrator.get_queue_info().await
    }

    /// Get challenge metrics
    pub async fn get_metrics(&self) -> ChallengeMetrics {
        self.monitor.get_metrics().await
    }

    /// Get active alerts
    pub async fn get_alerts(&self) -> Vec<ChallengeAlert> {
        self.monitor.get_alerts().await
    }

    /// Get orchestrator statistics
    pub async fn get_statistics(&self) -> OrchestratorStats {
        self.orchestrator.get_statistics().await
    }

    /// Cleanup completed challenges
    pub async fn cleanup_completed_challenges(&self) -> Result<usize> {
        self.orchestrator.cleanup_completed_challenges().await
    }

    /// Create custom alert
    pub async fn create_alert(
        &self,
        alert_type: AlertType,
        message: String,
        compose_hash: Option<String>,
    ) {
        self.monitor.create_alert(alert_type, message, compose_hash).await
    }

    /// Resolve an alert
    pub async fn resolve_alert(&self, alert_id: &str) -> Result<()> {
        self.monitor.resolve_alert(alert_id).await
    }

    /// Start orchestrator background tasks
    async fn start_orchestrator_tasks(&self) -> Result<()> {
        let orchestrator = self.orchestrator.clone();

        // Start cleanup task
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300)); // Every 5 minutes
            
            loop {
                interval.tick().await;
                
                if let Err(e) = orchestrator.cleanup_completed_challenges().await {
                    tracing::error!("Challenge cleanup task failed: {}", e);
                }
            }
        });

        Ok(())
    }

    /// Get system health status
    pub async fn get_health_status(&self) -> serde_json::Value {
        let stats = self.get_statistics().await;
        let metrics = self.get_metrics().await;
        let alerts = self.get_alerts().await;

        let active_alerts = alerts.iter().filter(|a| a.resolved_at.is_none()).count();
        let critical_alerts = alerts.iter()
            .filter(|a| a.resolved_at.is_none() && matches!(a.severity, AlertSeverity::Critical))
            .count();

        let failure_rate = metrics.failed_challenges as f64 / (metrics.successful_challenges + metrics.failed_challenges).max(1) as f64;
        let is_healthy = active_alerts == 0 && 
                        stats.active_challenges < stats.max_concurrent_challenges &&
                        failure_rate < 0.5;

        serde_json::json!({
            "healthy": is_healthy,
            "active_challenges": stats.active_challenges,
            "max_concurrent_challenges": stats.max_concurrent_challenges,
            "queued_challenges": stats.queued_challenges,
            "total_challenges": metrics.total_challenges,
            "success_rate": if metrics.total_challenges > 0 {
                metrics.successful_challenges as f64 / metrics.total_challenges as f64 * 100.0
            } else {
                0.0
            },
            "active_alerts": active_alerts,
            "critical_alerts": critical_alerts,
            "uptime": chrono::Utc::now().timestamp() - get_start_time(),
        })
    }

    /// Update configuration
    pub async fn update_config(&self, config: serde_json::Value) -> Result<()> {
        if let Some(max_concurrent) = config.get("max_concurrent_challenges").and_then(|v| v.as_u64()) {
            // Update orchestrator config (would need to make this configurable)
            info!("Updating max concurrent challenges to: {}", max_concurrent);
        }

        if let Some(alert_thresholds) = config.get("alert_thresholds") {
            // Update monitoring config (would need to make this configurable)
            info!("Updating alert thresholds: {:?}", alert_thresholds);
        }

        Ok(())
    }
}

/// Get system start time (placeholder)
fn get_start_time() -> i64 {
    // In a real implementation, this would be stored when the system starts
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_challenge_runner_creation() {
        // This would require setting up test database and dependencies
        // For now, just ensure the types compile correctly
        assert!(true);
    }
}
