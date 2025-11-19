//! CVM deployment and lifecycle management

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

use super::core::CvmDeploymentResult;

/// Deploy challenge CVM
pub async fn deploy_challenge_cvm(
    vmm_url: &str,
    gateway_url: &str,
    instance: &crate::challenge_runner::runner::types::ChallengeInstance,
) -> Result<CvmDeploymentResult> {
    info!(
        compose_hash = instance.compose_hash,
        "Deploying challenge CVM"
    );

    // Prepare deployment request
    let deployment_request = create_deployment_request(instance)?;

    // Send deployment request to VMM
    let response = send_deployment_request(vmm_url, &deployment_request).await?;

    // Parse deployment response
    let deployment_result = parse_deployment_response(response)?;

    info!(
        compose_hash = instance.compose_hash,
        instance_id = deployment_result.instance_id,
        "CVM deployed successfully"
    );

    Ok(deployment_result)
}

/// Stop CVM instance
pub async fn stop_cvm(vmm_url: &str, instance_id: &str) -> Result<()> {
    info!(instance_id = instance_id, "Stopping CVM instance");

    let stop_url = format!("{}/instances/{}/stop", vmm_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .post(&stop_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to send stop request")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to stop CVM: {}", error_text));
    }

    info!(instance_id = instance_id, "CVM instance stopped successfully");
    Ok(())
}

/// Restart CVM instance
pub async fn restart_cvm(vmm_url: &str, instance_id: &str) -> Result<()> {
    info!(instance_id = instance_id, "Restarting CVM instance");

    let restart_url = format!("{}/instances/{}/restart", vmm_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .post(&restart_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to send restart request")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to restart CVM: {}", error_text));
    }

    info!(instance_id = instance_id, "CVM instance restarted successfully");
    Ok(())
}

/// Create deployment request from challenge instance
fn create_deployment_request(
    instance: &crate::challenge_runner::runner::types::ChallengeInstance,
) -> Result<DeploymentRequest> {
    // Extract environment variables
    let mut env_vars = HashMap::new();
    
    // Add challenge-specific environment variables
    env_vars.insert("CHALLENGE_ID".to_string(), instance.challenge_id.clone());
    env_vars.insert("COMPOSE_HASH".to_string(), instance.compose_hash.clone());
    env_vars.insert("CHALLENGE_NAME".to_string(), instance.name.clone());
    env_vars.insert("SCHEMA_NAME".to_string(), instance.schema_name.clone());

    Ok(DeploymentRequest {
        image: instance.compose_hash.clone(),
        name: format!("challenge-{}", instance.compose_hash),
        env_vars,
        resources: ResourceSpec {
            cpu_cores: 2,
            memory_mb: 4096,
            disk_mb: 20480,
        },
        ports: vec![
            PortSpec {
                name: "api".to_string(),
                port: 8080,
                protocol: "tcp".to_string(),
            },
            PortSpec {
                name: "sdk".to_string(),
                port: 8081,
                protocol: "tcp".to_string(),
            },
        ],
        network: NetworkSpec {
            mode: "bridge".to_string(),
        },
        metadata: HashMap::new(),
    })
}

/// Send deployment request to VMM
async fn send_deployment_request(
    vmm_url: &str,
    request: &DeploymentRequest,
) -> Result<serde_json::Value> {
    let deploy_url = format!("{}/instances", vmm_url);

    let client = reqwest::Client::new();
    let response = client
        .post(&deploy_url)
        .header("Content-Type", "application/json")
        .json(request)
        .send()
        .await
        .context("Failed to send deployment request")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Deployment request failed: {}", error_text));
    }

    let response_json = response.json().await
        .context("Failed to parse deployment response")?;

    Ok(response_json)
}

/// Parse deployment response
fn parse_deployment_response(response: serde_json::Value) -> Result<CvmDeploymentResult> {
    // Parse response as Id struct (like validator does)
    #[derive(Deserialize)]
    struct Id {
        id: String,
    }

    let id: Id = serde_json::from_value(response)
        .context("Failed to parse deployment response ID")?;

    // For now, we'll use placeholder values
    // In a real implementation, you'd get these from the VMM response
    Ok(CvmDeploymentResult {
        instance_id: id.id,
        ip_address: "127.0.0.1".to_string(), // Placeholder
        api_port: 8080,
        sdk_port: 8081,
        status: "running".to_string(),
        created_at: chrono::Utc::now(),
    })
}

/// Deployment request structure
#[derive(Debug, Serialize)]
struct DeploymentRequest {
    image: String,
    name: String,
    env_vars: HashMap<String, String>,
    resources: ResourceSpec,
    ports: Vec<PortSpec>,
    network: NetworkSpec,
    metadata: HashMap<String, String>,
}

/// Resource specification
#[derive(Debug, Serialize)]
struct ResourceSpec {
    cpu_cores: u32,
    memory_mb: u32,
    disk_mb: u32,
}

/// Port specification
#[derive(Debug, Serialize)]
struct PortSpec {
    name: String,
    port: u16,
    protocol: String,
}

/// Network specification
#[derive(Debug, Serialize)]
struct NetworkSpec {
    mode: String,
}

/// Get deployment status
pub async fn get_deployment_status(vmm_url: &str, instance_id: &str) -> Result<DeploymentStatus> {
    debug!(instance_id = instance_id, "Getting deployment status");

    let status_url = format!("{}/instances/{}/status", vmm_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&status_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get deployment status")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get deployment status: {}", error_text));
    }

    let status: DeploymentStatus = response.json().await
        .context("Failed to parse deployment status")?;

    Ok(status)
}

/// Deployment status
#[derive(Debug, Deserialize)]
pub struct DeploymentStatus {
    pub instance_id: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub error_message: Option<String>,
}

/// Scale CVM instance
pub async fn scale_cvm(vmm_url: &str, instance_id: &str, replicas: u32) -> Result<()> {
    info!(
        instance_id = instance_id,
        replicas = replicas,
        "Scaling CVM instance"
    );

    let scale_url = format!("{}/instances/{}/scale", vmm_url, instance_id);

    #[derive(Serialize)]
    struct ScaleRequest {
        replicas: u32,
    }

    let scale_request = ScaleRequest { replicas };

    let client = reqwest::Client::new();
    let response = client
        .post(&scale_url)
        .header("Content-Type", "application/json")
        .json(&scale_request)
        .send()
        .await
        .context("Failed to send scale request")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to scale CVM: {}", error_text));
    }

    info!(
        instance_id = instance_id,
        replicas = replicas,
        "CVM instance scaled successfully"
    );

    Ok(())
}
