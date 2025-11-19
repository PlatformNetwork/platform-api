//! Core CVM management functionality

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

/// CVM deployment result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CvmDeploymentResult {
    pub instance_id: String,
    pub ip_address: String,
    pub api_port: u16,
    pub sdk_port: u16,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// CVM instance information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CvmInfo {
    pub instance_id: String,
    pub ip_address: String,
    pub api_port: u16,
    pub sdk_port: u16,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub metadata: HashMap<String, String>,
}

/// Core CVM manager
#[derive(Clone)]
pub struct CvmManager {
    vmm_url: String,
    gateway_url: String,
}

impl CvmManager {
    pub fn new(vmm_url: String, gateway_url: String) -> Self {
        Self { vmm_url, gateway_url }
    }

    /// Setup CVM environment for challenge
    pub async fn setup_cvm(
        &self,
        instance: &crate::challenge_runner::runner::types::ChallengeInstance,
    ) -> Result<CvmInfo> {
        info!(
            compose_hash = instance.compose_hash,
            "Setting up CVM environment"
        );

        // Deploy CVM
        let deployment_result = super::deployment::deploy_challenge_cvm(
            &self.vmm_url,
            &self.gateway_url,
            instance,
        ).await?;

        // Wait for CVM to be ready
        super::health_checker::wait_for_cvm_ready(
            &self.gateway_url,
            &deployment_result.instance_id,
        ).await?;

        // Send credentials
        super::credentials::send_credentials_to_cvm(
            &self.gateway_url,
            &deployment_result.instance_id,
            &instance.compose_hash,
        ).await?;

        let cvm_info = CvmInfo {
            instance_id: deployment_result.instance_id,
            ip_address: deployment_result.ip_address,
            api_port: deployment_result.api_port,
            sdk_port: deployment_result.sdk_port,
            status: deployment_result.status,
            created_at: deployment_result.created_at,
            metadata: HashMap::new(),
        };

        info!(
            compose_hash = instance.compose_hash,
            instance_id = cvm_info.instance_id,
            "CVM environment setup completed"
        );

        Ok(cvm_info)
    }

    /// Cleanup CVM environment
    pub async fn cleanup_cvm(&self, cvm_info: &CvmInfo) -> Result<()> {
        info!(
            instance_id = cvm_info.instance_id,
            "Cleaning up CVM environment"
        );

        super::deployment::stop_cvm(&self.vmm_url, &cvm_info.instance_id).await?;

        info!(
            instance_id = cvm_info.instance_id,
            "CVM environment cleanup completed"
        );

        Ok(())
    }

    /// Check CVM health
    pub async fn check_cvm_health(&self, instance_id: &str) -> Result<bool> {
        debug!(instance_id = instance_id, "Checking CVM health");
        super::health_checker::check_cvm_health(&self.gateway_url, instance_id).await
    }

    /// Get CVM information
    pub async fn get_cvm_info(&self, instance_id: &str) -> Result<CvmInfo> {
        debug!(instance_id = instance_id, "Getting CVM information");
        super::info::get_cvm_info(&self.vmm_url, instance_id).await
    }

    /// Get migrations from CVM
    pub async fn get_migrations_from_cvm(&self, cvm_api_url: &str) -> Result<Vec<super::migration::Migration>> {
        debug!(cvm_api_url = cvm_api_url, "Getting migrations from CVM");
        super::migration::get_migrations_from_cvm(cvm_api_url).await
    }

    /// List all CVM instances
    pub async fn list_cvm_instances(&self) -> Result<Vec<CvmInfo>> {
        debug!("Listing all CVM instances");
        super::info::list_cvm_instances(&self.vmm_url).await
    }

    /// Restart CVM instance
    pub async fn restart_cvm(&self, instance_id: &str) -> Result<()> {
        info!(instance_id = instance_id, "Restarting CVM instance");
        super::deployment::restart_cvm(&self.vmm_url, instance_id).await
    }

    /// Get CVM logs
    pub async fn get_cvm_logs(&self, instance_id: &str, lines: Option<u32>) -> Result<String> {
        debug!(
            instance_id = instance_id,
            lines = ?lines,
            "Getting CVM logs"
        );
        super::logging::get_cvm_logs(&self.gateway_url, instance_id, lines).await
    }

    /// Execute command in CVM
    pub async fn execute_cvm_command(
        &self,
        instance_id: &str,
        command: &str,
    ) -> Result<CvmCommandResult> {
        debug!(
            instance_id = instance_id,
            command = command,
            "Executing CVM command"
        );
        super::execution::execute_cvm_command(&self.gateway_url, instance_id, command).await
    }

    /// Get CVM resource usage
    pub async fn get_cvm_resource_usage(&self, instance_id: &str) -> Result<CvmResourceUsage> {
        debug!(instance_id = instance_id, "Getting CVM resource usage");
        super::monitoring::get_cvm_resource_usage(&self.gateway_url, instance_id).await
    }
}

/// CVM command execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CvmCommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub execution_time_ms: u64,
}

/// CVM resource usage information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CvmResourceUsage {
    pub cpu_usage_percent: f64,
    pub memory_usage_mb: u64,
    pub disk_usage_mb: u64,
    pub network_bytes_sent: u64,
    pub network_bytes_received: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}
