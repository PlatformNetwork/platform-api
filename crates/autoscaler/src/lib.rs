use anyhow::Result;
use platform_api_models::*;
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

/// Autoscaler service for managing VM pools
pub struct AutoscalerService {
    config: AutoscalerConfig,
    vms: tokio::sync::RwLock<HashMap<String, VmInstance>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmInstance {
    pub vm_id: String,
    pub pool_id: Uuid,
    pub node_id: Uuid,
    pub status: VmStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmStatus {
    Creating,
    Running,
    Stopping,
    Stopped,
    Failed,
}

impl AutoscalerService {
    pub fn new(config: &AutoscalerConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            vms: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Scale up a pool by creating VMs
    pub async fn scale_up(&self, pool_id: Uuid, target_count: u32) -> Result<ScalePoolResponse> {
        info!("Scaling up pool {} to {} VMs", pool_id, target_count);

        let mut vms = self.vms.write().await;
        let previous_count = vms.len() as u32;
        let mut created_vms = Vec::new();

        // Create VMs via dstack VMM API
        for i in 0..target_count {
            let vm_id = format!("vm-{}-{}", pool_id, i);
            let vm_instance = VmInstance {
                vm_id: vm_id.clone(),
                pool_id,
                node_id: Uuid::new_v4(),
                status: VmStatus::Creating,
                created_at: chrono::Utc::now(),
            };

            // Create VM instance (integration with dstack VMM would be added here)
            // Currently tracking VM instances for testing
            vms.insert(vm_id.clone(), vm_instance.clone());
            created_vms.push(vm_id.clone());

            info!("Created VM instance: {}", vm_id);
        }

        Ok(ScalePoolResponse {
            pool_id,
            previous_nodes: previous_count,
            target_nodes: target_count,
            actual_nodes: vms.len() as u32,
            created_vms,
            removed_vms: Vec::new(),
        })
    }

    /// Scale down a pool by stopping VMs
    pub async fn scale_down(&self, pool_id: Uuid, target_count: u32) -> Result<ScalePoolResponse> {
        info!("Scaling down pool {} to {} VMs", pool_id, target_count);

        let mut vms = self.vms.write().await;
        let previous_count = vms.len() as u32;
        let mut removed_vms = Vec::new();

        // Get VMs for this pool
        let pool_vms: Vec<String> = vms
            .values()
            .filter(|vm| vm.pool_id == pool_id)
            .map(|vm| vm.vm_id.clone())
            .collect();

        // Remove excess VMs
        let to_remove = pool_vms.len().saturating_sub(target_count as usize);
        for (i, vm_id) in pool_vms.iter().enumerate() {
            if i < to_remove {
                // Shutdown VM instance (integration with dstack VMM would be added here)
                vms.remove(vm_id);
                removed_vms.push(vm_id.clone());
                info!("Removed VM instance: {}", vm_id);
            }
        }

        Ok(ScalePoolResponse {
            pool_id,
            previous_nodes: previous_count,
            target_nodes: target_count,
            actual_nodes: vms.len() as u32,
            created_vms: Vec::new(),
            removed_vms,
        })
    }

    /// List VMs for a pool
    pub async fn list_vms(&self, pool_id: Uuid) -> Result<Vec<VmInstance>> {
        let vms = self.vms.read().await;
        Ok(vms
            .values()
            .filter(|vm| vm.pool_id == pool_id)
            .cloned()
            .collect())
    }

    /// Get VM status
    pub async fn get_vm_status(&self, vm_id: &str) -> Result<VmInstance> {
        let vms = self.vms.read().await;
        vms.get(vm_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("VM not found"))
    }
}

#[derive(Debug, Clone)]
pub struct AutoscalerConfig {
    pub enabled: bool,
    pub scale_up_delay: u64,
    pub scale_down_delay: u64,
    pub min_idle_vms: u32,
    pub max_idle_vms: u32,
}

impl Default for AutoscalerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scale_up_delay: 60,
            scale_down_delay: 300,
            min_idle_vms: 1,
            max_idle_vms: 10,
        }
    }
}
