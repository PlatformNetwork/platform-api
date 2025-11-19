//! Challenge monitoring and metrics collection

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use super::runner_core::ChallengeRunnerCore;
use super::challenge_orchestrator::{ChallengeOrchestrator, ChallengeExecution};

/// Monitoring system for challenge execution
pub struct ChallengeMonitor {
    core: Arc<ChallengeRunnerCore>,
    orchestrator: Arc<ChallengeOrchestrator>,
    metrics: Arc<RwLock<ChallengeMetrics>>,
    alerts: Arc<RwLock<Vec<ChallengeAlert>>>,
    config: MonitoringConfig,
}

/// Configuration for monitoring system
#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    pub metrics_retention_hours: u64,
    pub alert_thresholds: AlertThresholds,
    pub health_check_interval_seconds: u64,
    pub enable_performance_tracking: bool,
}

/// Alert thresholds for monitoring
#[derive(Debug, Clone)]
pub struct AlertThresholds {
    pub max_execution_time_minutes: u64,
    pub max_failure_rate_percent: f64,
    pub max_memory_usage_mb: u64,
    pub max_cpu_usage_percent: f64,
}

/// Collected challenge metrics
#[derive(Debug, Default, Serialize)]
pub struct ChallengeMetrics {
    pub total_challenges: u64,
    pub successful_challenges: u64,
    pub failed_challenges: u64,
    pub average_execution_time_seconds: f64,
    pub total_execution_time_seconds: f64,
    pub peak_memory_usage_mb: u64,
    pub average_memory_usage_mb: f64,
    pub peak_cpu_usage_percent: f64,
    pub average_cpu_usage_percent: f64,
    pub challenges_by_status: HashMap<String, u64>,
    pub execution_history: Vec<ExecutionRecord>,
    pub performance_history: Vec<PerformanceSnapshot>,
}

/// Individual execution record
#[derive(Debug, Serialize)]
pub struct ExecutionRecord {
    pub compose_hash: String,
    pub challenge_id: String,
    pub status: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub execution_time_seconds: Option<f64>,
    pub peak_memory_usage_mb: Option<u64>,
    pub average_cpu_usage_percent: Option<f64>,
    pub error_message: Option<String>,
}

/// Performance snapshot at a point in time
#[derive(Debug, Serialize)]
pub struct PerformanceSnapshot {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub active_challenges: usize,
    pub queued_challenges: usize,
    pub system_memory_usage_mb: u64,
    pub system_cpu_usage_percent: f64,
    pub network_io_bytes_per_second: f64,
}

/// Alert for challenge issues
#[derive(Debug, Serialize)]
pub struct ChallengeAlert {
    pub id: String,
    pub alert_type: AlertType,
    pub severity: AlertSeverity,
    pub message: String,
    pub compose_hash: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Types of alerts
#[derive(Debug, Serialize)]
pub enum AlertType {
    ChallengeTimeout,
    HighFailureRate,
    ResourceExhaustion,
    SystemOverload,
    ExecutionError,
}

/// Alert severity levels
#[derive(Debug, Serialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

impl ChallengeMonitor {
    /// Create new challenge monitor
    pub fn new(
        core: Arc<ChallengeRunnerCore>,
        orchestrator: Arc<ChallengeOrchestrator>,
        config: MonitoringConfig,
    ) -> Self {
        Self {
            core,
            orchestrator,
            metrics: Arc::new(RwLock::new(ChallengeMetrics::default())),
            alerts: Arc::new(RwLock::new(Vec::new())),
            config,
        }
    }

    /// Start monitoring background tasks
    pub async fn start_monitoring(&self) -> Result<()> {
        info!("Starting challenge monitoring system");

        // Start metrics collection
        self.start_metrics_collection().await;

        // Start health checks
        self.start_health_checks().await;

        // Start alert processing
        self.start_alert_processing().await;

        info!("Challenge monitoring system started");
        Ok(())
    }

    /// Record challenge execution start
    pub async fn record_challenge_start(&self, compose_hash: &str, challenge_id: &str) {
        debug!("Recording challenge start: {}", compose_hash);

        let record = ExecutionRecord {
            compose_hash: compose_hash.to_string(),
            challenge_id: challenge_id.to_string(),
            status: "running".to_string(),
            started_at: chrono::Utc::now(),
            completed_at: None,
            execution_time_seconds: None,
            peak_memory_usage_mb: None,
            average_cpu_usage_percent: None,
            error_message: None,
        };

        let mut metrics = self.metrics.write().await;
        metrics.total_challenges += 1;
        metrics.execution_history.push(record);
        
        // Update status counts
        *metrics.challenges_by_status.entry("running".to_string()).or_insert(0) += 1;
    }

    /// Record challenge execution completion
    pub async fn record_challenge_completion(
        &self,
        compose_hash: &str,
        success: bool,
        execution_time_seconds: f64,
        error_message: Option<String>,
    ) {
        debug!("Recording challenge completion: {} (success: {})", compose_hash, success);

        let mut metrics = self.metrics.write().await;
        
        // Update execution history
        if let Some(record) = metrics.execution_history.iter_mut().find(|r| r.compose_hash == compose_hash) {
            record.status = if success { "completed".to_string() } else { "failed".to_string() };
            record.completed_at = Some(chrono::Utc::now());
            record.execution_time_seconds = Some(execution_time_seconds);
            record.error_message = error_message;
        }

        // Update counters
        if success {
            metrics.successful_challenges += 1;
            *metrics.challenges_by_status.entry("completed".to_string()).or_insert(0) += 1;
        } else {
            metrics.failed_challenges += 1;
            *metrics.challenges_by_status.entry("failed".to_string()).or_insert(0) += 1;
            
            // Check for high failure rate
            self.check_failure_rate(&metrics).await;
        }

        // Update execution time metrics
        metrics.total_execution_time_seconds += execution_time_seconds;
        metrics.average_execution_time_seconds = metrics.total_execution_time_seconds / 
            (metrics.successful_challenges + metrics.failed_challenges) as f64;
    }

    /// Record resource usage for a challenge
    pub async fn record_resource_usage(
        &self,
        compose_hash: &str,
        memory_usage_mb: u64,
        cpu_usage_percent: f64,
    ) {
        debug!("Recording resource usage for {}: {} MB, {}% CPU", compose_hash, memory_usage_mb, cpu_usage_percent);

        let mut metrics = self.metrics.write().await;
        
        // Update peak values
        metrics.peak_memory_usage_mb = metrics.peak_memory_usage_mb.max(memory_usage_mb);
        metrics.peak_cpu_usage_percent = metrics.peak_cpu_usage_percent.max(cpu_usage_percent);
        
        // Update average values (simplified calculation)
        let total_recorded = metrics.execution_history.len() as f64;
        metrics.average_memory_usage_mb = 
            (metrics.average_memory_usage_mb * (total_recorded - 1.0) + memory_usage_mb as f64) / total_recorded;
        metrics.average_cpu_usage_percent = 
            (metrics.average_cpu_usage_percent * (total_recorded - 1.0) + cpu_usage_percent) / total_recorded;

        // Update execution record
        if let Some(record) = metrics.execution_history.iter_mut().find(|r| r.compose_hash == compose_hash) {
            record.peak_memory_usage_mb = Some(memory_usage_mb);
            record.average_cpu_usage_percent = Some(cpu_usage_percent);
        }

        // Check for resource alerts
        self.check_resource_alerts(memory_usage_mb, cpu_usage_percent).await;
    }

    /// Get current metrics
    pub async fn get_metrics(&self) -> ChallengeMetrics {
        self.metrics.read().await.clone()
    }

    /// Get active alerts
    pub async fn get_alerts(&self) -> Vec<ChallengeAlert> {
        self.alerts.read().await.clone()
    }

    /// Create new alert
    pub async fn create_alert(&self, alert_type: AlertType, message: String, compose_hash: Option<String>) {
        let alert = ChallengeAlert {
            id: uuid::Uuid::new_v4().to_string(),
            alert_type,
            severity: AlertSeverity::Warning, // Default severity
            message,
            compose_hash,
            created_at: chrono::Utc::now(),
            resolved_at: None,
        };

        let mut alerts = self.alerts.write().await;
        alerts.push(alert);

        warn!("Created new challenge alert");
    }

    /// Resolve an alert
    pub async fn resolve_alert(&self, alert_id: &str) -> Result<()> {
        let mut alerts = self.alerts.write().await;
        if let Some(alert) = alerts.iter_mut().find(|a| a.id == alert_id) {
            alert.resolved_at = Some(chrono::Utc::now());
            info!("Resolved alert: {}", alert_id);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Alert not found: {}", alert_id))
        }
    }

    /// Start metrics collection background task
    async fn start_metrics_collection(&self) {
        let metrics = self.metrics.clone();
        let orchestrator = self.orchestrator.clone();
        let interval = std::time::Duration::from_secs(60); // Collect every minute

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            
            loop {
                ticker.tick().await;
                
                let snapshot = collect_performance_snapshot(&orchestrator).await;
                
                let mut metrics = metrics.write().await;
                metrics.performance_history.push(snapshot);
                
                // Keep only last 24 hours of snapshots
                let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);
                metrics.performance_history.retain(|s| s.timestamp > cutoff);
            }
        });
    }

    /// Start health check background task
    async fn start_health_checks(&self) {
        let core = self.core.clone();
        let orchestrator = self.orchestrator.clone();
        let monitor = self.metrics.clone();
        let alert_thresholds = self.config.alert_thresholds.clone();
        let interval = std::time::Duration::from_secs(self.config.health_check_interval_seconds);

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            
            loop {
                ticker.tick().await;
                
                if let Err(e) = perform_health_checks(&core, &orchestrator, &monitor, &alert_thresholds).await {
                    error!("Health check failed: {}", e);
                }
            }
        });
    }

    /// Start alert processing background task
    async fn start_alert_processing(&self) {
        let alerts = self.alerts.clone();
        let interval = std::time::Duration::from_secs(300); // Process every 5 minutes

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            
            loop {
                ticker.tick().await;
                
                // Clean up old resolved alerts
                let mut alerts_guard = alerts.write().await;
                let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);
                alerts_guard.retain(|a| {
                    a.resolved_at.is_none() || a.resolved_at.unwrap() > cutoff
                });
            }
        });
    }

    /// Check for high failure rate
    async fn check_failure_rate(&self, metrics: &ChallengeMetrics) {
        let total_executed = metrics.successful_challenges + metrics.failed_challenges;
        if total_executed > 10 { // Only check after sufficient sample size
            let failure_rate = (metrics.failed_challenges as f64 / total_executed as f64) * 100.0;
            
            if failure_rate > self.config.alert_thresholds.max_failure_rate_percent {
                self.create_alert(
                    AlertType::HighFailureRate,
                    format!("High failure rate detected: {:.1}%", failure_rate),
                    None,
                ).await;
            }
        }
    }

    /// Check for resource usage alerts
    async fn check_resource_alerts(&self, memory_usage_mb: u64, cpu_usage_percent: f64) {
        if memory_usage_mb > self.config.alert_thresholds.max_memory_usage_mb {
            self.create_alert(
                AlertType::ResourceExhaustion,
                format!("High memory usage detected: {} MB", memory_usage_mb),
                None,
            ).await;
        }

        if cpu_usage_percent > self.config.alert_thresholds.max_cpu_usage_percent {
            self.create_alert(
                AlertType::ResourceExhaustion,
                format!("High CPU usage detected: {:.1}%", cpu_usage_percent),
                None,
            ).await;
        }
    }
}

/// Collect performance snapshot
async fn collect_performance_snapshot(orchestrator: &Arc<ChallengeOrchestrator>) -> PerformanceSnapshot {
    let stats = orchestrator.get_statistics().await;
    
    // Get system metrics (simplified)
    let system_memory_usage_mb = get_system_memory_usage().await.unwrap_or(0);
    let system_cpu_usage_percent = get_system_cpu_usage().await.unwrap_or(0.0);
    let network_io_bytes_per_second = get_network_io_rate().await.unwrap_or(0.0);

    PerformanceSnapshot {
        timestamp: chrono::Utc::now(),
        active_challenges: stats.active_challenges,
        queued_challenges: stats.queued_challenges,
        system_memory_usage_mb,
        system_cpu_usage_percent,
        network_io_bytes_per_second,
    }
}

/// Perform health checks
async fn perform_health_checks(
    core: &Arc<ChallengeRunnerCore>,
    orchestrator: &Arc<ChallengeOrchestrator>,
    metrics: &Arc<RwLock<ChallengeMetrics>>,
    alert_thresholds: &AlertThresholds,
) -> Result<()> {
    // Check for stuck challenges
    let all_status = orchestrator.get_all_challenges_status().await;
    let now = chrono::Utc::now();
    
    for (compose_hash, status) in all_status {
        if let Some(execution) = orchestrator.get_challenge_execution(&compose_hash).await {
            let running_duration = now.signed_duration_since(execution.started_at);
            let running_minutes = running_duration.num_minutes() as u64;
            
            if running_minutes > alert_thresholds.max_execution_time_minutes {
                warn!("Challenge {} running for {} minutes (threshold: {})", 
                      compose_hash, running_minutes, alert_thresholds.max_execution_time_minutes);
                
                // Could create an alert here
            }
        }
    }

    Ok(())
}

/// Get system memory usage (placeholder implementation)
async fn get_system_memory_usage() -> Result<u64> {
    // In a real implementation, you'd query system metrics
    Ok(1024) // 1GB placeholder
}

/// Get system CPU usage (placeholder implementation)
async fn get_system_cpu_usage() -> Result<f64> {
    // In a real implementation, you'd query system metrics
    Ok(50.0) // 50% placeholder
}

/// Get network I/O rate (placeholder implementation)
async fn get_network_io_rate() -> Result<f64> {
    // In a real implementation, you'd query network metrics
    Ok(1024.0) // 1KB/s placeholder
}
