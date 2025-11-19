//! CVM health checking and monitoring

use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Wait for CVM to be ready
pub async fn wait_for_cvm_ready(gateway_url: &str, instance_id: &str) -> Result<()> {
    info!(
        instance_id = instance_id,
        "Waiting for CVM to be ready"
    );

    let timeout = Duration::from_secs(300); // 5 minutes timeout
    let start_time = std::time::Instant::now();
    let mut check_interval = Duration::from_secs(5);

    while start_time.elapsed() < timeout {
        match check_cvm_health(gateway_url, instance_id).await {
            Ok(true) => {
                info!(
                    instance_id = instance_id,
                    elapsed_seconds = start_time.elapsed().as_secs(),
                    "CVM is ready"
                );
                return Ok(());
            }
            Ok(false) => {
                debug!(
                    instance_id = instance_id,
                    elapsed_seconds = start_time.elapsed().as_secs(),
                    "CVM not ready yet, waiting..."
                );
            }
            Err(e) => {
                warn!(
                    instance_id = instance_id,
                    error = %e,
                    "Health check failed, retrying..."
                );
            }
        }

        // Exponential backoff
        tokio::time::sleep(check_interval).await;
        check_interval = std::cmp::min(check_interval * 2, Duration::from_secs(30));
    }

    error!(
        instance_id = instance_id,
        timeout_seconds = timeout.as_secs(),
        "CVM readiness timeout"
    );

    Err(anyhow::anyhow!("CVM failed to become ready within timeout"))
}

/// Check CVM health
pub async fn check_cvm_health(gateway_url: &str, instance_id: &str) -> Result<bool> {
    debug!(instance_id = instance_id, "Checking CVM health");

    let health_url = format!("{}/instances/{}/health", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&health_url)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .context("Failed to send health check request")?;

    match response.status() {
        reqwest::StatusCode::OK => {
            debug!(instance_id = instance_id, "CVM health check passed");
            Ok(true)
        }
        reqwest::StatusCode::SERVICE_UNAVAILABLE => {
            debug!(instance_id = instance_id, "CVM not ready yet");
            Ok(false)
        }
        status => {
            let error_text = response.text().await
                .context("Failed to read error response")?;
            warn!(
                instance_id = instance_id,
                status = %status,
                error = %error_text,
                "Unexpected health check response"
            );
            Ok(false)
        }
    }
}

/// Get detailed health information
pub async fn get_cvm_health_info(gateway_url: &str, instance_id: &str) -> Result<CvmHealthInfo> {
    debug!(instance_id = instance_id, "Getting CVM health information");

    let health_url = format!("{}/instances/{}/health/detailed", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&health_url)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .context("Failed to get health info")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get health info: {}", error_text));
    }

    let health_info: CvmHealthInfo = response.json().await
        .context("Failed to parse health info")?;

    debug!(
        instance_id = instance_id,
        status = %health_info.status,
        uptime_seconds = health_info.uptime_seconds,
        "Retrieved CVM health information"
    );

    Ok(health_info)
}

/// Perform comprehensive health check
pub async fn perform_comprehensive_health_check(
    gateway_url: &str,
    instance_id: &str,
) -> Result<HealthCheckResult> {
    info!(instance_id = instance_id, "Performing comprehensive health check");

    let mut result = HealthCheckResult {
        instance_id: instance_id.to_string(),
        overall_healthy: true,
        checks: HashMap::new(),
        timestamp: chrono::Utc::now(),
    };

    // Basic health check
    match check_cvm_health(gateway_url, instance_id).await {
        Ok(healthy) => {
            result.checks.insert("basic".to_string(), HealthCheck {
                name: "Basic Health".to_string(),
                status: if healthy { "healthy".to_string() } else { "unhealthy".to_string() },
                message: if healthy { "CVM is responding" } else { "CVM is not responding" }.to_string(),
                duration_ms: 0,
            });
            if !healthy {
                result.overall_healthy = false;
            }
        }
        Err(e) => {
            result.checks.insert("basic".to_string(), HealthCheck {
                name: "Basic Health".to_string(),
                status: "error".to_string(),
                message: format!("Health check failed: {}", e),
                duration_ms: 0,
            });
            result.overall_healthy = false;
        }
    }

    // API endpoint check
    let start_time = std::time::Instant::now();
    match check_api_endpoint(gateway_url, instance_id).await {
        Ok(_) => {
            result.checks.insert("api".to_string(), HealthCheck {
                name: "API Endpoint".to_string(),
                status: "healthy".to_string(),
                message: "API endpoint is accessible".to_string(),
                duration_ms: start_time.elapsed().as_millis() as u64,
            });
        }
        Err(e) => {
            result.checks.insert("api".to_string(), HealthCheck {
                name: "API Endpoint".to_string(),
                status: "error".to_string(),
                message: format!("API check failed: {}", e),
                duration_ms: start_time.elapsed().as_millis() as u64,
            });
            result.overall_healthy = false;
        }
    }

    // Resource check
    match check_resource_usage(gateway_url, instance_id).await {
        Ok(usage) => {
            let status = if usage.cpu_usage_percent < 90.0 && usage.memory_usage_mb < 8000 {
                "healthy"
            } else {
                "warning"
            };
            
            result.checks.insert("resources".to_string(), HealthCheck {
                name: "Resource Usage".to_string(),
                status: status.to_string(),
                message: format!("CPU: {:.1}%, Memory: {} MB", usage.cpu_usage_percent, usage.memory_usage_mb),
                duration_ms: 0,
            });
        }
        Err(e) => {
            result.checks.insert("resources".to_string(), HealthCheck {
                name: "Resource Usage".to_string(),
                status: "error".to_string(),
                message: format!("Resource check failed: {}", e),
                duration_ms: 0,
            });
            result.overall_healthy = false;
        }
    }

    info!(
        instance_id = instance_id,
        overall_healthy = result.overall_healthy,
        check_count = result.checks.len(),
        "Comprehensive health check completed"
    );

    Ok(result)
}

/// Check API endpoint availability
async fn check_api_endpoint(gateway_url: &str, instance_id: &str) -> Result<()> {
    let api_url = format!("{}/instances/{}/api/health", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&api_url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .context("Failed to check API endpoint")?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("API endpoint returned status: {}", response.status()))
    }
}

/// Check resource usage
async fn check_resource_usage(gateway_url: &str, instance_id: &str) -> Result<ResourceUsage> {
    let usage_url = format!("{}/instances/{}/resources", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&usage_url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .context("Failed to get resource usage")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Resource usage endpoint failed"));
    }

    let usage: ResourceUsage = response.json().await
        .context("Failed to parse resource usage")?;

    Ok(usage)
}

/// CVM health information
#[derive(Debug, Deserialize)]
pub struct CvmHealthInfo {
    pub status: String,
    pub uptime_seconds: u64,
    pub version: String,
    pub last_check: chrono::DateTime<chrono::Utc>,
    pub checks: HashMap<String, serde_json::Value>,
}

/// Health check result
#[derive(Debug, serde::Serialize)]
pub struct HealthCheckResult {
    pub instance_id: String,
    pub overall_healthy: bool,
    pub checks: HashMap<String, HealthCheck>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Individual health check
#[derive(Debug, serde::Serialize)]
pub struct HealthCheck {
    pub name: String,
    pub status: String,
    pub message: String,
    pub duration_ms: u64,
}

/// Resource usage information
#[derive(Debug, Deserialize)]
pub struct ResourceUsage {
    pub cpu_usage_percent: f64,
    pub memory_usage_mb: u64,
    pub disk_usage_mb: u64,
    pub network_bytes_sent: u64,
    pub network_bytes_received: u64,
}
