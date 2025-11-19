//! CVM information and listing functionality

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use super::core::CvmInfo;

/// Get CVM information
pub async fn get_cvm_info(vmm_url: &str, instance_id: &str) -> Result<CvmInfo> {
    debug!(instance_id = instance_id, "Getting CVM information");

    let info_url = format!("{}/instances/{}", vmm_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&info_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM info")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM info: {}", error_text));
    }

    let vm_info: VmInfo = response.json().await
        .context("Failed to parse CVM info")?;

    let cvm_info = CvmInfo {
        instance_id: vm_info.id,
        ip_address: vm_info.ip_address,
        api_port: vm_info.api_port,
        sdk_port: vm_info.sdk_port,
        status: vm_info.status,
        created_at: vm_info.created_at,
        metadata: vm_info.metadata,
    };

    debug!(
        instance_id = instance_id,
        status = cvm_info.status,
        "Retrieved CVM information"
    );

    Ok(cvm_info)
}

/// List all CVM instances
pub async fn list_cvm_instances(vmm_url: &str) -> Result<Vec<CvmInfo>> {
    debug!("Listing all CVM instances");

    let list_url = format!("{}/instances", vmm_url);

    let client = reqwest::Client::new();
    let response = client
        .get(&list_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to list CVM instances")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to list CVM instances: {}", error_text));
    }

    let vm_list: Vec<VmInfo> = response.json().await
        .context("Failed to parse CVM list")?;

    let cvm_instances: Vec<CvmInfo> = vm_list
        .into_iter()
        .map(|vm_info| CvmInfo {
            instance_id: vm_info.id,
            ip_address: vm_info.ip_address,
            api_port: vm_info.api_port,
            sdk_port: vm_info.sdk_port,
            status: vm_info.status,
            created_at: vm_info.created_at,
            metadata: vm_info.metadata,
        })
        .collect();

    info!(
        instance_count = cvm_instances.len(),
        "Listed all CVM instances"
    );

    Ok(cvm_instances)
}

/// Get CVM statistics
pub async fn get_cvm_statistics(vmm_url: &str) -> Result<CvmStatistics> {
    debug!("Getting CVM statistics");

    let stats_url = format!("{}/instances/stats", vmm_url);

    let client = reqwest::Client::new();
    let response = client
        .get(&stats_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM statistics")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM statistics: {}", error_text));
    }

    let stats: CvmStatistics = response.json().await
        .context("Failed to parse CVM statistics")?;

    debug!(
        total_instances = stats.total_instances,
        running_instances = stats.running_instances,
        "Retrieved CVM statistics"
    );

    Ok(stats)
}

/// Search CVM instances by criteria
pub async fn search_cvm_instances(
    vmm_url: &str,
    search_criteria: &CvmSearchCriteria,
) -> Result<Vec<CvmInfo>> {
    debug!(
        status_filter = ?search_criteria.status,
        "Searching CVM instances"
    );

    let search_url = format!("{}/instances/search", vmm_url);

    let client = reqwest::Client::new();
    let response = client
        .post(&search_url)
        .header("Content-Type", "application/json")
        .json(search_criteria)
        .send()
        .await
        .context("Failed to search CVM instances")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to search CVM instances: {}", error_text));
    }

    let vm_list: Vec<VmInfo> = response.json().await
        .context("Failed to parse search results")?;

    let cvm_instances: Vec<CvmInfo> = vm_list
        .into_iter()
        .map(|vm_info| CvmInfo {
            instance_id: vm_info.id,
            ip_address: vm_info.ip_address,
            api_port: vm_info.api_port,
            sdk_port: vm_info.sdk_port,
            status: vm_info.status,
            created_at: vm_info.created_at,
            metadata: vm_info.metadata,
        })
        .collect();

    info!(
        search_results = cvm_instances.len(),
        "CVM instance search completed"
    );

    Ok(cvm_instances)
}

/// Get CVM instance logs
pub async fn get_cvm_instance_logs(
    vmm_url: &str,
    instance_id: &str,
    lines: Option<u32>,
    follow: bool,
) -> Result<String> {
    debug!(
        instance_id = instance_id,
        lines = ?lines,
        follow = follow,
        "Getting CVM instance logs"
    );

    let mut logs_url = format!("{}/instances/{}/logs", vmm_url, instance_id);
    
    let mut query_params = Vec::new();
    if let Some(lines) = lines {
        query_params.push(format!("lines={}", lines));
    }
    if follow {
        query_params.push("follow=true".to_string());
    }
    
    if !query_params.is_empty() {
        logs_url.push('?');
        logs_url.push_str(&query_params.join("&"));
    }

    let client = reqwest::Client::new();
    let response = client
        .get(&logs_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM logs")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM logs: {}", error_text));
    }

    let logs = response.text().await
        .context("Failed to read logs")?;

    debug!(
        instance_id = instance_id,
        log_length = logs.len(),
        "Retrieved CVM instance logs"
    );

    Ok(logs)
}

/// Get CVM instance events
pub async fn get_cvm_instance_events(
    vmm_url: &str,
    instance_id: &str,
    since: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Vec<CvmEvent>> {
    debug!(
        instance_id = instance_id,
        since = ?since,
        "Getting CVM instance events"
    );

    let mut events_url = format!("{}/instances/{}/events", vmm_url, instance_id);
    
    if let Some(since) = since {
        events_url.push_str(&format!("?since={}", since.timestamp()));
    }

    let client = reqwest::Client::new();
    let response = client
        .get(&events_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM events")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM events: {}", error_text));
    }

    let events: Vec<CvmEvent> = response.json().await
        .context("Failed to parse CVM events")?;

    debug!(
        instance_id = instance_id,
        event_count = events.len(),
        "Retrieved CVM instance events"
    );

    Ok(events)
}

/// VM information from VMM
#[derive(Debug, Deserialize)]
struct VmInfo {
    id: String,
    ip_address: String,
    api_port: u16,
    sdk_port: u16,
    status: String,
    created_at: chrono::DateTime<chrono::Utc>,
    metadata: HashMap<String, String>,
}

/// CVM statistics
#[derive(Debug, Deserialize)]
pub struct CvmStatistics {
    pub total_instances: u64,
    pub running_instances: u64,
    pub stopped_instances: u64,
    pub failed_instances: u64,
    pub total_memory_usage_mb: u64,
    pub total_cpu_usage_percent: f64,
    pub average_uptime_hours: f64,
}

/// CVM search criteria
#[derive(Debug, Serialize)]
pub struct CvmSearchCriteria {
    pub status: Option<String>,
    pub created_after: Option<chrono::DateTime<chrono::Utc>>,
    pub created_before: Option<chrono::DateTime<chrono::Utc>>,
    pub metadata_filters: HashMap<String, String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// CVM event
#[derive(Debug, Deserialize)]
pub struct CvmEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub event_type: String,
    pub message: String,
    pub severity: String,
    pub metadata: HashMap<String, String>,
}

/// CVM instance configuration
#[derive(Debug, Deserialize)]
pub struct CvmConfig {
    pub instance_id: String,
    pub image: String,
    pub resources: CvmResources,
    pub network: CvmNetworkConfig,
    pub environment: HashMap<String, String>,
    pub startup_command: Option<String>,
}

/// CVM resource configuration
#[derive(Debug, Deserialize)]
pub struct CvmResources {
    pub cpu_cores: u32,
    pub memory_mb: u32,
    pub disk_mb: u32,
}

/// CVM network configuration
#[derive(Debug, Deserialize)]
pub struct CvmNetworkConfig {
    pub mode: String,
    pub ports: Vec<CvmPortConfig>,
}

/// CVM port configuration
#[derive(Debug, Deserialize)]
pub struct CvmPortConfig {
    pub name: String,
    pub port: u16,
    pub protocol: String,
    pub public: bool,
}

/// Get CVM configuration
pub async fn get_cvm_config(vmm_url: &str, instance_id: &str) -> Result<CvmConfig> {
    debug!(instance_id = instance_id, "Getting CVM configuration");

    let config_url = format!("{}/instances/{}/config", vmm_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&config_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get CVM config")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get CVM config: {}", error_text));
    }

    let config: CvmConfig = response.json().await
        .context("Failed to parse CVM config")?;

    debug!(
        instance_id = instance_id,
        image = config.image,
        "Retrieved CVM configuration"
    );

    Ok(config)
}
