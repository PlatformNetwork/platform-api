use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Validator pool for managing TEE nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pool {
    pub id: Uuid,
    pub validator_hotkey: String,
    pub name: String,
    pub description: Option<String>,
    pub autoscale_policy: AutoscalePolicy,
    pub region: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Autoscaling policy for a pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoscalePolicy {
    pub min_nodes: u32,
    pub max_nodes: u32,
    pub target_cpu_utilization: f64,
    pub target_memory_utilization: f64,
    pub scale_up_threshold: f64,
    pub scale_down_threshold: f64,
    pub scale_up_cooldown: u64,
    pub scale_down_cooldown: u64,
}

impl Default for AutoscalePolicy {
    fn default() -> Self {
        Self {
            min_nodes: 1,
            max_nodes: 10,
            target_cpu_utilization: 0.7,
            target_memory_utilization: 0.7,
            scale_up_threshold: 0.8,
            scale_down_threshold: 0.5,
            scale_up_cooldown: 60,
            scale_down_cooldown: 300,
        }
    }
}

/// TEE node in a pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: Uuid,
    pub pool_id: Uuid,
    pub name: String,
    pub vmm_url: String,
    pub capacity: NodeCapacity,
    pub health: NodeHealth,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Node capacity information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapacity {
    pub total_cpu: u32,
    pub total_memory_gb: u32,
    pub total_disk_gb: u32,
    pub available_cpu: u32,
    pub available_memory_gb: u32,
    pub available_disk_gb: u32,
    pub has_tdx: bool,
    pub has_gpu: bool,
    pub gpu_count: u32,
    pub gpu_model: Option<String>,
    pub region: String,
}

impl Default for NodeCapacity {
    fn default() -> Self {
        Self {
            total_cpu: 8,
            total_memory_gb: 32,
            total_disk_gb: 100,
            available_cpu: 8,
            available_memory_gb: 32,
            available_disk_gb: 100,
            has_tdx: true,
            has_gpu: false,
            gpu_count: 0,
            gpu_model: None,
            region: "us-east-1".to_string(),
        }
    }
}

/// Node health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHealth {
    pub status: HealthStatus,
    pub last_heartbeat: DateTime<Utc>,
    pub vmm_reachable: bool,
    pub guest_agent_reachable: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

impl Default for NodeHealth {
    fn default() -> Self {
        Self {
            status: HealthStatus::Unknown,
            last_heartbeat: Utc::now(),
            vmm_reachable: false,
            guest_agent_reachable: false,
            error: None,
        }
    }
}

/// Request to create a pool
#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePoolRequest {
    pub name: String,
    pub description: Option<String>,
    pub autoscale_policy: Option<AutoscalePolicy>,
    pub region: Option<String>,
}

/// Request to update a pool
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdatePoolRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub autoscale_policy: Option<AutoscalePolicy>,
    pub region: Option<String>,
}

/// Request to add a node to a pool
#[derive(Debug, Serialize, Deserialize)]
pub struct AddNodeRequest {
    pub name: String,
    pub vmm_url: String,
    pub capacity: NodeCapacity,
    pub metadata: Option<serde_json::Value>,
}

/// Request to update a node
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateNodeRequest {
    pub name: Option<String>,
    pub vmm_url: Option<String>,
    pub capacity: Option<NodeCapacity>,
    pub metadata: Option<serde_json::Value>,
}

/// Response for pool list
#[derive(Debug, Serialize, Deserialize)]
pub struct PoolListResponse {
    pub pools: Vec<Pool>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

/// Response for node list
#[derive(Debug, Serialize, Deserialize)]
pub struct NodeListResponse {
    pub nodes: Vec<Node>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

/// Request to scale a pool
#[derive(Debug, Serialize, Deserialize)]
pub struct ScalePoolRequest {
    pub target_nodes: u32,
    pub reason: Option<String>,
}

/// Response for scale operation
#[derive(Debug, Serialize, Deserialize)]
pub struct ScalePoolResponse {
    pub pool_id: Uuid,
    pub previous_nodes: u32,
    pub target_nodes: u32,
    pub actual_nodes: u32,
    pub created_vms: Vec<String>,
    pub removed_vms: Vec<String>,
}

/// Pool capacity summary
#[derive(Debug, Serialize, Deserialize)]
pub struct PoolCapacitySummary {
    pub pool_id: Uuid,
    pub total_nodes: u32,
    pub healthy_nodes: u32,
    pub total_cpu: u32,
    pub available_cpu: u32,
    pub total_memory_gb: u32,
    pub available_memory_gb: u32,
    pub has_tdx: bool,
    pub gpu_count: u32,
}
