//! Refactored CVM manager module with organized components

pub mod core;
pub mod deployment;
pub mod health_checker;
pub mod credentials;
pub mod migration;
pub mod info;
pub mod logging;
pub mod execution;
pub mod monitoring;

use anyhow::Result;

pub use core::{
    CvmManager, CvmDeploymentResult, CvmInfo, CvmCommandResult, CvmResourceUsage,
};
pub use deployment::{DeploymentStatus};
pub use health_checker::{CvmHealthInfo, HealthCheckResult, HealthCheck};
pub use credentials::{CredentialStatus, CredentialAuditLog};
pub use migration::{Migration, CvmMigrationStatus, MigrationValidationResult, MigrationHistoryEntry};
pub use info::{CvmStatistics, CvmSearchCriteria, CvmEvent, CvmConfig};
pub use logging::{LogEntry, LogFilter, LogStatistics, LogSearchQuery};
pub use execution::{ExecutionOptions, CommandHistoryEntry, RunningCommandInfo};
pub use monitoring::{CvmMetrics, TimeRange, CvmAlert, AlertThresholds, CvmPerformanceSummary};

/// Create CVM manager with all components
pub fn create_cvm_manager(vmm_url: String, gateway_url: String) -> CvmManager {
    CvmManager::new(vmm_url, gateway_url)
}

/// Initialize CVM management system
pub async fn initialize_cvm_system() -> Result<()> {
    // Ensure required directories exist
    std::fs::create_dir_all("cvm_logs")
        .map_err(|e| anyhow::anyhow!("Failed to create CVM logs directory: {}", e))?;
    
    std::fs::create_dir_all("cvm_backups")
        .map_err(|e| anyhow::anyhow!("Failed to create CVM backups directory: {}", e))?;

    info!("CVM management system initialized");
    Ok(())
}

/// CVM utilities
pub struct CvmUtils;

impl CvmUtils {
    /// Validate CVM configuration
    pub async fn validate_cvm_config(config: &CvmConfig) -> Result<()> {
        // Validate resource limits
        if config.resources.cpu_cores == 0 {
            return Err(anyhow::anyhow!("CPU cores must be greater than 0"));
        }
        
        if config.resources.memory_mb < 512 {
            return Err(anyhow::anyhow!("Memory must be at least 512 MB"));
        }
        
        if config.resources.disk_mb < 1024 {
            return Err(anyhow::anyhow!("Disk must be at least 1 GB"));
        }

        // Validate network configuration
        if config.network.ports.is_empty() {
            return Err(anyhow::anyhow!("At least one port must be configured"));
        }

        // Validate port ranges
        for port_config in &config.network.ports {
            if port_config.port < 1024 && port_config.public {
                return Err(anyhow::anyhow!("Public ports must be >= 1024"));
            }
        }

        info!("CVM configuration validation passed");
        Ok(())
    }

    /// Generate CVM instance name
    pub fn generate_instance_name(challenge_id: &str) -> String {
        format!("cvm-{}-{}", challenge_id, chrono::Utc::now().timestamp())
    }

    /// Calculate estimated resource usage
    pub fn estimate_resource_usage(config: &CvmConfig) -> ResourceEstimate {
        ResourceEstimate {
            estimated_cpu_cores: config.resources.cpu_cores,
            estimated_memory_mb: config.resources.memory_mb,
            estimated_disk_mb: config.resources.disk_mb,
            estimated_network_mbps: 100, // Default estimate
            estimated_cost_per_hour: calculate_hourly_cost(config),
        }
    }

    /// Get CVM health score
    pub async fn calculate_health_score(
        gateway_url: &str,
        instance_id: &str,
    ) -> Result<HealthScore> {
        let metrics = monitoring::get_cvm_metrics(gateway_url, instance_id).await?;
        let resource_usage = monitoring::get_cvm_resource_usage(gateway_url, instance_id).await?;
        let alerts = monitoring::get_cvm_alerts(gateway_url, instance_id, None).await?;

        let mut score = 100.0;

        // CPU score (40% weight)
        if resource_usage.cpu_usage_percent > 90.0 {
            score -= 40.0;
        } else if resource_usage.cpu_usage_percent > 70.0 {
            score -= 20.0;
        } else if resource_usage.cpu_usage_percent > 50.0 {
            score -= 10.0;
        }

        // Memory score (30% weight)
        let memory_usage_percent = (resource_usage.memory_usage_mb as f64 / 8192.0) * 100.0; // Assuming 8GB max
        if memory_usage_percent > 90.0 {
            score -= 30.0;
        } else if memory_usage_percent > 70.0 {
            score -= 15.0;
        } else if memory_usage_percent > 50.0 {
            score -= 7.5;
        }

        // Alerts score (20% weight)
        let critical_alerts = alerts.iter().filter(|a| a.severity == "critical").count() as f64;
        let warning_alerts = alerts.iter().filter(|a| a.severity == "warning").count() as f64;
        score -= (critical_alerts * 20.0) + (warning_alerts * 5.0);

        // Uptime score (10% weight)
        let uptime_hours = metrics.uptime_seconds as f64 / 3600.0;
        if uptime_hours < 1.0 {
            score -= 10.0;
        } else if uptime_hours < 24.0 {
            score -= 5.0;
        }

        score = score.max(0.0).min(100.0);

        Ok(HealthScore {
            overall_score: score,
            cpu_score: calculate_cpu_score(resource_usage.cpu_usage_percent),
            memory_score: calculate_memory_score(memory_usage_percent),
            uptime_score: calculate_uptime_score(uptime_hours),
            alert_score: calculate_alert_score(critical_alerts, warning_alerts),
            calculated_at: chrono::Utc::now(),
        })
    }
}

/// Resource usage estimate
#[derive(Debug, serde::Serialize)]
pub struct ResourceEstimate {
    pub estimated_cpu_cores: u32,
    pub estimated_memory_mb: u32,
    pub estimated_disk_mb: u32,
    pub estimated_network_mbps: u32,
    pub estimated_cost_per_hour: f64,
}

/// Health score breakdown
#[derive(Debug, serde::Serialize)]
pub struct HealthScore {
    pub overall_score: f64,
    pub cpu_score: f64,
    pub memory_score: f64,
    pub uptime_score: f64,
    pub alert_score: f64,
    pub calculated_at: chrono::DateTime<chrono::Utc>,
}

/// Calculate hourly cost for CVM
fn calculate_hourly_cost(config: &CvmConfig) -> f64 {
    // Simple cost calculation (would be based on actual pricing in production)
    let cpu_cost = config.resources.cpu_cores as f64 * 0.05; // $0.05 per CPU core per hour
    let memory_cost = (config.resources.memory_mb as f64 / 1024.0) * 0.01; // $0.01 per GB per hour
    let disk_cost = (config.resources.disk_mb as f64 / 1024.0) * 0.001; // $0.001 per GB per hour
    
    cpu_cost + memory_cost + disk_cost
}

/// Calculate CPU score component
fn calculate_cpu_score(cpu_usage_percent: f64) -> f64 {
    if cpu_usage_percent > 90.0 {
        0.0
    } else if cpu_usage_percent > 70.0 {
        25.0
    } else if cpu_usage_percent > 50.0 {
        35.0
    } else {
        40.0
    }
}

/// Calculate memory score component
fn calculate_memory_score(memory_usage_percent: f64) -> f64 {
    if memory_usage_percent > 90.0 {
        0.0
    } else if memory_usage_percent > 70.0 {
        15.0
    } else if memory_usage_percent > 50.0 {
        22.5
    } else {
        30.0
    }
}

/// Calculate uptime score component
fn calculate_uptime_score(uptime_hours: f64) -> f64 {
    if uptime_hours < 1.0 {
        0.0
    } else if uptime_hours < 24.0 {
        5.0
    } else {
        10.0
    }
}

/// Calculate alert score component
fn calculate_alert_score(critical_alerts: f64, warning_alerts: f64) -> f64 {
    let score = 20.0 - (critical_alerts * 20.0) - (warning_alerts * 5.0);
    score.max(0.0)
}

/// CVM performance benchmark
pub async fn run_cvm_benchmark(
    gateway_url: &str,
    instance_id: &str,
    benchmark_type: &str,
) -> Result<BenchmarkResult> {
    info!(
        instance_id = instance_id,
        benchmark_type = benchmark_type,
        "Running CVM benchmark"
    );

    match benchmark_type {
        "cpu" => run_cpu_benchmark(gateway_url, instance_id).await,
        "memory" => run_memory_benchmark(gateway_url, instance_id).await,
        "disk" => run_disk_benchmark(gateway_url, instance_id).await,
        "network" => run_network_benchmark(gateway_url, instance_id).await,
        "comprehensive" => run_comprehensive_benchmark(gateway_url, instance_id).await,
        _ => Err(anyhow::anyhow!("Unknown benchmark type: {}", benchmark_type)),
    }
}

/// Run CPU benchmark
async fn run_cpu_benchmark(gateway_url: &str, instance_id: &str) -> Result<BenchmarkResult> {
    let command = "sysbench cpu --cpu-max-prime=10000 run";
    let result = execution::execute_cvm_command(gateway_url, instance_id, command).await?;
    
    Ok(BenchmarkResult {
        benchmark_type: "cpu".to_string(),
        score: parse_cpu_score(&result.stdout),
        execution_time_ms: result.execution_time_ms,
        details: result.stdout,
        timestamp: chrono::Utc::now(),
    })
}

/// Run memory benchmark
async fn run_memory_benchmark(gateway_url: &str, instance_id: &str) -> Result<BenchmarkResult> {
    let command = "sysbench memory --memory-block-size=1K --memory-total-size=1G run";
    let result = execution::execute_cvm_command(gateway_url, instance_id, command).await?;
    
    Ok(BenchmarkResult {
        benchmark_type: "memory".to_string(),
        score: parse_memory_score(&result.stdout),
        execution_time_ms: result.execution_time_ms,
        details: result.stdout,
        timestamp: chrono::Utc::now(),
    })
}

/// Run disk benchmark
async fn run_disk_benchmark(gateway_url: &str, instance_id: &str) -> Result<BenchmarkResult> {
    let command = "sysbench fileio --file-test-mode=rndrw --file-size=1G run";
    let result = execution::execute_cvm_command(gateway_url, instance_id, command).await?;
    
    Ok(BenchmarkResult {
        benchmark_type: "disk".to_string(),
        score: parse_disk_score(&result.stdout),
        execution_time_ms: result.execution_time_ms,
        details: result.stdout,
        timestamp: chrono::Utc::now(),
    })
}

/// Run network benchmark
async fn run_network_benchmark(gateway_url: &str, instance_id: &str) -> Result<BenchmarkResult> {
    let command = "ping -c 10 8.8.8.8";
    let result = execution::execute_cvm_command(gateway_url, instance_id, command).await?;
    
    Ok(BenchmarkResult {
        benchmark_type: "network".to_string(),
        score: parse_network_score(&result.stdout),
        execution_time_ms: result.execution_time_ms,
        details: result.stdout,
        timestamp: chrono::Utc::now(),
    })
}

/// Run comprehensive benchmark
async fn run_comprehensive_benchmark(gateway_url: &str, instance_id: &str) -> Result<BenchmarkResult> {
    let mut results = Vec::new();
    
    results.push(run_cpu_benchmark(gateway_url, instance_id).await?);
    results.push(run_memory_benchmark(gateway_url, instance_id).await?);
    results.push(run_disk_benchmark(gateway_url, instance_id).await?);
    results.push(run_network_benchmark(gateway_url, instance_id).await?);
    
    let total_score: f64 = results.iter().map(|r| r.score).sum();
    let average_score = total_score / results.len() as f64;
    let total_execution_time: u64 = results.iter().map(|r| r.execution_time_ms).sum();
    
    Ok(BenchmarkResult {
        benchmark_type: "comprehensive".to_string(),
        score: average_score,
        execution_time_ms: total_execution_time,
        details: serde_json::to_string(&results).unwrap_or_default(),
        timestamp: chrono::Utc::now(),
    })
}

/// Benchmark result
#[derive(Debug, serde::Serialize)]
pub struct BenchmarkResult {
    pub benchmark_type: String,
    pub score: f64,
    pub execution_time_ms: u64,
    pub details: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Parse CPU score from benchmark output
fn parse_cpu_score(output: &str) -> f64 {
    // Simple parsing - would need more sophisticated parsing in production
    if output.contains("events per second") {
        // Extract events per second and convert to score
        1000.0 // Placeholder
    } else {
        0.0
    }
}

/// Parse memory score from benchmark output
fn parse_memory_score(output: &str) -> f64 {
    // Simple parsing - would need more sophisticated parsing in production
    if output.contains("MiB/sec") {
        1000.0 // Placeholder
    } else {
        0.0
    }
}

/// Parse disk score from benchmark output
fn parse_disk_score(output: &str) -> f64 {
    // Simple parsing - would need more sophisticated parsing in production
    if output.contains("MiB/sec") {
        1000.0 // Placeholder
    } else {
        0.0
    }
}

/// Parse network score from benchmark output
fn parse_network_score(output: &str) -> f64 {
    // Simple parsing - would need more sophisticated parsing in production
    if output.contains("avg") {
        1000.0 // Placeholder
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instance_name_generation() {
        let name = CvmUtils::generate_instance_name("test-challenge");
        assert!(name.starts_with("cvm-test-challenge-"));
    }

    #[test]
    fn test_resource_estimation() {
        let config = CvmConfig {
            instance_id: "test".to_string(),
            image: "test-image".to_string(),
            resources: CvmResources {
                cpu_cores: 2,
                memory_mb: 4096,
                disk_mb: 20480,
            },
            network: CvmNetworkConfig {
                mode: "bridge".to_string(),
                ports: vec![],
            },
            environment: std::collections::HashMap::new(),
            startup_command: None,
        };

        let estimate = CvmUtils::estimate_resource_usage(&config);
        assert_eq!(estimate.estimated_cpu_cores, 2);
        assert_eq!(estimate.estimated_memory_mb, 4096);
        assert!(estimate.estimated_cost_per_hour > 0.0);
    }
}
