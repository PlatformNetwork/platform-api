//! CVM monitoring and resource tracking

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

use super::core::CvmResourceUsage;

/// Get CVM resource usage
pub async fn get_cvm_resource_usage(gateway_url: &str, instance_id: &str) -> Result<CvmResourceUsage> {
    debug!(instance_id = instance_id, "Getting CVM resource usage");

    let usage_url = format!("{}/instances/{}/resources", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&usage_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM resource usage")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM resource usage: {}", error_text));
    }

    let usage: ResourceUsageResponse = response.json().await
        .context("Failed to parse resource usage")?;

    let resource_usage = CvmResourceUsage {
        cpu_usage_percent: usage.cpu_usage_percent,
        memory_usage_mb: usage.memory_usage_mb,
        disk_usage_mb: usage.disk_usage_mb,
        network_bytes_sent: usage.network_bytes_sent,
        network_bytes_received: usage.network_bytes_received,
        timestamp: usage.timestamp,
    };

    debug!(
        instance_id = instance_id,
        cpu_percent = resource_usage.cpu_usage_percent,
        memory_mb = resource_usage.memory_usage_mb,
        "Retrieved CVM resource usage"
    );

    Ok(resource_usage)
}

/// Get detailed CVM metrics
pub async fn get_cvm_metrics(gateway_url: &str, instance_id: &str) -> Result<CvmMetrics> {
    debug!(instance_id = instance_id, "Getting detailed CVM metrics");

    let metrics_url = format!("{}/instances/{}/metrics", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&metrics_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM metrics")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM metrics: {}", error_text));
    }

    let metrics: CvmMetrics = response.json().await
        .context("Failed to parse CVM metrics")?;

    debug!(
        instance_id = instance_id,
        uptime_seconds = metrics.uptime_seconds,
        process_count = metrics.process_count,
        "Retrieved detailed CVM metrics"
    );

    Ok(metrics)
}

/// Get CVM performance history
pub async fn get_cvm_performance_history(
    gateway_url: &str,
    instance_id: &str,
    time_range: &TimeRange,
    interval_seconds: u32,
) -> Result<Vec<CvmResourceUsage>> {
    debug!(
        instance_id = instance_id,
        start_time = ?time_range.start,
        end_time = ?time_range.end,
        interval_seconds = interval_seconds,
        "Getting CVM performance history"
    );

    let history_url = format!(
        "{}/instances/{}/metrics/history?start={}&end={}&interval={}",
        gateway_url,
        instance_id,
        time_range.start.timestamp(),
        time_range.end.timestamp(),
        interval_seconds
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&history_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM performance history")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM performance history: {}", error_text));
    }

    let history: Vec<CvmResourceUsage> = response.json().await
        .context("Failed to parse performance history")?;

    debug!(
        instance_id = instance_id,
        data_points = history.len(),
        "Retrieved CVM performance history"
    );

    Ok(history)
}

/// Monitor CVM in real-time
pub async fn monitor_cvm_realtime(
    gateway_url: &str,
    instance_id: &str,
) -> Result<tokio::sync::mpsc::Receiver<CvmResourceUsage>> {
    info!(instance_id = instance_id, "Starting real-time CVM monitoring");

    let (tx, rx) = tokio::sync::mpsc::channel(1000);

    let monitor_url = format!("{}/instances/{}/metrics/stream", gateway_url, instance_id);

    // Spawn monitoring task
    let instance_id = instance_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = monitoring_task(monitor_url, tx).await {
            error!(
                instance_id = instance_id,
                error = %e,
                "CVM monitoring task failed"
            );
        }
    });

    Ok(rx)
}

/// Monitoring background task
async fn monitoring_task(
    monitor_url: String,
    tx: tokio::sync::mpsc::Sender<CvmResourceUsage>,
) -> Result<()> {
    use futures_util::StreamExt;

    let client = reqwest::Client::new();
    let response = client
        .get(&monitor_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to start monitoring stream")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Monitoring stream failed: {}", response.status()));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("Failed to read monitoring chunk")?;
        
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        
        // Process complete JSON objects
        while let Some(brace_pos) = buffer.find('}') {
            let json_str = buffer[..=brace_pos].to_string();
            buffer = buffer[brace_pos + 1..].to_string();

            if let Ok(resource_usage) = parse_resource_usage_json(&json_str) {
                if tx.send(resource_usage).await.is_err() {
                    // Channel closed, stop monitoring
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Parse resource usage JSON
fn parse_resource_usage_json(json_str: &str) -> Result<CvmResourceUsage> {
    let resource_response: ResourceUsageResponse = serde_json::from_str(json_str)
        .context("Failed to parse resource usage JSON")?;

    Ok(CvmResourceUsage {
        cpu_usage_percent: resource_response.cpu_usage_percent,
        memory_usage_mb: resource_response.memory_usage_mb,
        disk_usage_mb: resource_response.disk_usage_mb,
        network_bytes_sent: resource_response.network_bytes_sent,
        network_bytes_received: resource_response.network_bytes_received,
        timestamp: resource_response.timestamp,
    })
}

/// Get CVM alerts
pub async fn get_cvm_alerts(
    gateway_url: &str,
    instance_id: &str,
    severity: Option<String>,
) -> Result<Vec<CvmAlert>> {
    debug!(
        instance_id = instance_id,
        severity = ?severity,
        "Getting CVM alerts"
    );

    let mut alerts_url = format!("{}/instances/{}/alerts", gateway_url, instance_id);
    
    if let Some(severity) = severity {
        alerts_url.push_str(&format!("?severity={}", severity));
    }

    let client = reqwest::Client::new();
    let response = client
        .get(&alerts_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM alerts")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM alerts: {}", error_text));
    }

    let alerts: Vec<CvmAlert> = response.json().await
        .context("Failed to parse CVM alerts")?;

    debug!(
        instance_id = instance_id,
        alert_count = alerts.len(),
        "Retrieved CVM alerts"
    );

    Ok(alerts)
}

/// Set CVM alert thresholds
pub async fn set_cvm_alert_thresholds(
    gateway_url: &str,
    instance_id: &str,
    thresholds: &AlertThresholds,
) -> Result<()> {
    info!(
        instance_id = instance_id,
        "Setting CVM alert thresholds"
    );

    let thresholds_url = format!("{}/instances/{}/alerts/thresholds", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .post(&thresholds_url)
        .header("Content-Type", "application/json")
        .json(thresholds)
        .send()
        .await
        .context("Failed to set alert thresholds")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to set alert thresholds: {}", error_text));
    }

    info!(
        instance_id = instance_id,
        "CVM alert thresholds set successfully"
    );

    Ok(())
}

/// Resource usage response from CVM
#[derive(Debug, Deserialize)]
struct ResourceUsageResponse {
    pub cpu_usage_percent: f64,
    pub memory_usage_mb: u64,
    pub disk_usage_mb: u64,
    pub network_bytes_sent: u64,
    pub network_bytes_received: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Detailed CVM metrics
#[derive(Debug, Deserialize)]
pub struct CvmMetrics {
    pub uptime_seconds: u64,
    pub process_count: u32,
    pub thread_count: u32,
    pub file_descriptor_count: u32,
    pub load_average: [f64; 3], // 1min, 5min, 15min
    pub context_switches: u64,
    pub page_faults: u64,
    pub disk_io_reads: u64,
    pub disk_io_writes: u64,
    pub network_connections: u32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Time range for historical data
#[derive(Debug, Serialize)]
pub struct TimeRange {
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
}

/// CVM alert
#[derive(Debug, Deserialize)]
pub struct CvmAlert {
    pub id: String,
    pub alert_type: String,
    pub severity: String,
    pub message: String,
    pub threshold_value: f64,
    pub actual_value: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Alert thresholds configuration
#[derive(Debug, Serialize)]
pub struct AlertThresholds {
    pub cpu_warning_percent: f64,
    pub cpu_critical_percent: f64,
    pub memory_warning_mb: u64,
    pub memory_critical_mb: u64,
    pub disk_warning_percent: f64,
    pub disk_critical_percent: f64,
    pub network_error_rate_threshold: f64,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            cpu_warning_percent: 70.0,
            cpu_critical_percent: 90.0,
            memory_warning_mb: 6144, // 6GB
            memory_critical_mb: 7680, // 7.5GB
            disk_warning_percent: 80.0,
            disk_critical_percent: 95.0,
            network_error_rate_threshold: 0.05, // 5%
        }
    }
}

/// Get CVM performance summary
pub async fn get_cvm_performance_summary(
    gateway_url: &str,
    instance_id: &str,
    period_hours: u32,
) -> Result<CvmPerformanceSummary> {
    debug!(
        instance_id = instance_id,
        period_hours = period_hours,
        "Getting CVM performance summary"
    );

    let summary_url = format!(
        "{}/instances/{}/metrics/summary?period_hours={}",
        gateway_url, instance_id, period_hours
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&summary_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get performance summary")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get performance summary: {}", error_text));
    }

    let summary: CvmPerformanceSummary = response.json().await
        .context("Failed to parse performance summary")?;

    debug!(
        instance_id = instance_id,
        avg_cpu_percent = summary.average_cpu_percent,
        avg_memory_mb = summary.average_memory_mb,
        "Retrieved CVM performance summary"
    );

    Ok(summary)
}

/// CVM performance summary
#[derive(Debug, Deserialize)]
pub struct CvmPerformanceSummary {
    pub period_hours: u32,
    pub average_cpu_percent: f64,
    pub peak_cpu_percent: f64,
    pub average_memory_mb: u64,
    pub peak_memory_mb: u64,
    pub average_disk_usage_mb: u64,
    pub peak_disk_usage_mb: u64,
    pub total_network_bytes: u64,
    pub uptime_percentage: f64,
    pub alert_count: u32,
    pub data_points: u32,
}
