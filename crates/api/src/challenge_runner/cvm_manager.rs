use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml;
use std::collections::HashMap;
use tracing::{debug, info, warn};

use super::migrations::Migration;

/// Result of CVM deployment
#[derive(Debug, Clone)]
pub struct CvmDeploymentResult {
    pub instance_id: String,
    pub api_url: String,
}

/// CVM Manager for deploying and managing challenge CVMs
pub struct CvmManager {
    vmm_url: String,
    gateway_url: String,
    http_client: reqwest::Client,
    mock_mode: bool,
}

impl CvmManager {
    pub fn new(vmm_url: String, gateway_url: String) -> Self {
        // Check if mock mode is enabled (for dev environment)
        let mock_mode = std::env::var("PLATFORM_API_MOCK_VMM")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true"
            || std::env::var("DEV_MODE")
                .unwrap_or_else(|_| "false".to_string())
                .to_lowercase()
                == "true";

        if mock_mode {
            debug!("MOCK VMM MODE: CVM operations will use Docker services directly");
        }

        Self {
            vmm_url,
            gateway_url,
            http_client: reqwest::Client::new(),
            mock_mode,
        }
    }

    /// Deploy a challenge CVM (API mode) via dstack VMM using docker-compose runner
    pub async fn deploy_challenge_cvm(
        &self,
        compose_hash: &str,
        challenge_id: &str,
        challenge_name: &str,
        image: &str,
        compose_yaml: &str,
        env_vars: Option<&HashMap<String, String>>,
    ) -> Result<CvmDeploymentResult> {
        info!(
            compose_hash = compose_hash,
            challenge_id = challenge_id,
            challenge_name = challenge_name,
            image = image,
            "Deploying challenge CVM (API mode)"
        );

        // Generate short UUID from compose_hash (first 12 characters)
        let short_hash = &compose_hash[..compose_hash.len().min(12)];
        let cvm_name = format!("api-{}-{}", challenge_name, short_hash);

        // Check if CVM with this name already exists
        if let Some(existing_vm_id) = self.check_cvm_exists(&cvm_name).await? {
            info!(
                cvm_name = &cvm_name,
                compose_hash = compose_hash,
                existing_vm_id = existing_vm_id,
                "CVM with name already exists, will use existing"
            );
            // Get gateway URL for existing CVM
            let (base_domain, gateway_port) = self.get_gateway_meta().await?;

            if self.mock_mode {
                // In mock mode, use Docker service URL directly
                let api_url = format!("http://{}:{}", base_domain, gateway_port);
                return Ok(CvmDeploymentResult {
                    instance_id: existing_vm_id,
                    api_url,
                });
            }

            let instance_id = self.get_instance_id(&existing_vm_id).await.unwrap_or(None);
            let host = if let Some(id) = instance_id.as_ref() {
                if !id.trim().is_empty() {
                    format!("{}-10000.{}:{}", id, base_domain, gateway_port)
                } else {
                    format!("-10000.{}:{}", base_domain, gateway_port)
                }
            } else {
                format!("-10000.{}:{}", base_domain, gateway_port)
            };
            let api_url = format!("https://{}", host);
            return Ok(CvmDeploymentResult {
                instance_id: existing_vm_id,
                api_url,
            });
        }

        // In mock mode, use Docker service directly instead of creating VMM CVM
        if self.mock_mode && compose_hash.contains("term-challenge") {
            let (base_domain, gateway_port) = self.get_gateway_meta().await?;
            let api_url = format!("http://{}:{}", base_domain, gateway_port);
            return Ok(CvmDeploymentResult {
                instance_id: "term-challenge-dev-docker".to_string(),
                api_url,
            });
        }

        // Parse docker-compose YAML from database
        let mut compose_doc: serde_yaml::Value = serde_yaml::from_str(compose_yaml)
            .context("Failed to parse docker-compose YAML from database")?;

        // Inject CHALLENGE_ADMIN=true and CHALLENGE_ID for API mode (admin mode)
        // Find the main service (usually the first service or named after the challenge)
        if let Some(services) = compose_doc
            .get_mut("services")
            .and_then(|s| s.as_mapping_mut())
        {
            // First, find the service name we want to modify
            let service_name_opt = services
                .keys()
                .find(|k| {
                    let name = k.as_str().unwrap_or("");
                    name == challenge_name || name == "challenge" || name == "app"
                })
                .or_else(|| services.keys().next())
                .and_then(|k| k.as_str().map(|s| s.to_string()));

            // Now modify the service
            if let Some(service_name) = service_name_opt {
                if let Some(service) = services.get_mut(&service_name) {
                    // Ensure environment section exists
                    if service.get("environment").is_none() {
                        service["environment"] = serde_yaml::Value::Sequence(Vec::new());
                    }

                    // Add or update CHALLENGE_ADMIN and CHALLENGE_ID
                    if let Some(env) = service
                        .get_mut("environment")
                        .and_then(|e| e.as_sequence_mut())
                    {
                        // Remove existing CHALLENGE_ADMIN, SDK_RUN_SERVER (legacy), and CHALLENGE_ID if present
                        env.retain(|v| {
                            if let Some(s) = v.as_str() {
                                !s.starts_with("CHALLENGE_ADMIN=")
                                    && !s.starts_with("SDK_RUN_SERVER=")
                                    && !s.starts_with("CHALLENGE_ID=")
                            } else {
                                true
                            }
                        });
                        // Add new values
                        env.push(serde_yaml::Value::String("CHALLENGE_ADMIN=true".to_string()));
                        env.push(serde_yaml::Value::String(format!(
                            "CHALLENGE_ID={}",
                            challenge_id
                        )));

                        // Add challenge environment variables from database
                        if let Some(env_vars_map) = env_vars {
                            for (key, value) in env_vars_map {
                                // Skip if already present (shouldn't happen, but be safe)
                                let env_entry = format!("{}={}", key, value);
                                if !env.iter().any(|v| {
                                    v.as_str()
                                        .map(|s| s.starts_with(&format!("{}=", key)))
                                        .unwrap_or(false)
                                }) {
                                    env.push(serde_yaml::Value::String(env_entry));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Convert back to YAML string
        let modified_compose_yaml = serde_yaml::to_string(&compose_doc)
            .context("Failed to serialize modified docker-compose YAML")?;

        info!("Using docker-compose from database (modified for API mode)");

        // Compose manifest expected by dstack VMM
        // Use production-ready defaults matching validator expectations
        let compose_manifest = serde_json::json!({
            "manifest_version": 2,
            "name": cvm_name.clone(),
            "runner": "docker-compose",
            "docker_compose_file": modified_compose_yaml,
            "docker_config": {}, // Empty docker config by default
            "kms_enabled": true, // Enable KMS by default for Update Compose button
            "gateway_enabled": true,
            "public_logs": true, // Enable logs by default
            "public_sysinfo": true, // Enable sysinfo by default
            "public_tcbinfo": true,
            "local_key_provider_enabled": false,
            "key_provider_id": "",
            "allowed_envs": [],
            "no_instance_id": false,
            "secure_time": false // Default to false for production
        });

        // Serialize compose manifest to JSON string (like validator does)
        let compose_file_str = serde_json::to_string(&compose_manifest)
            .context("Failed to serialize compose manifest to JSON")?;

        // Create VM via dstack VMM PRPC - use api-{name}-{short_hash} as name
        let create_vm_payload = serde_json::json!({
            "name": cvm_name,
            "image": image,
            "compose_file": compose_file_str,
            "vcpu": 4,
            "memory": 4096,
            "disk_size": 10240,
            "ports": [],
            "encrypted_env": "",
            "user_config": "",
            "hugepages": false,
            "pin_numa": false,
            "stopped": false
        });

        let create_url = format!("{}/prpc/CreateVm?json", self.vmm_url);

        // Log request payload for debugging (like validator does)
        info!(
            "Sending CreateVm request to VMM: {}",
            serde_json::to_string_pretty(&create_vm_payload)?
        );

        let response = self
            .http_client
            .post(&create_url)
            .json(&create_vm_payload)
            .send()
            .await
            .context("Failed to send CreateVm request")?;

        let status = response.status();
        info!("VMM response status: {}", status);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(anyhow::anyhow!(
                "VMM API error ({}): {}",
                status,
                error_text
            ));
        }

        // Get response body for debugging (like validator does)
        let response_body = response.text().await?;
        info!("VMM response body: {}", response_body);

        // Parse response as Id struct (like validator does)
        #[derive(Deserialize)]
        struct Id {
            id: String,
        }
        let vm_id: Id = serde_json::from_str(&response_body)
            .with_context(|| format!("Failed to parse VMM response '{}'", response_body))?;

        info!("Created VM with ID: {}", vm_id.id);

        // Fetch VMM gateway metadata (base_domain, port)
        let (base_domain, gateway_port) = self.get_gateway_meta().await?;

        // Try to resolve instance_id immediately; may be empty early
        let instance_id = self.get_instance_id(&vm_id.id).await.unwrap_or(None);

        // Build public API URL as the validator does
        let host = if let Some(id) = instance_id.as_ref() {
            if !id.trim().is_empty() {
                format!("{}-10000.{}:{}", id, base_domain, gateway_port)
            } else {
                format!("-10000.{}:{}", base_domain, gateway_port)
            }
        } else {
            format!("-10000.{}:{}", base_domain, gateway_port)
        };
        let api_url = format!("https://{}", host);

        Ok(CvmDeploymentResult {
            instance_id: vm_id.id,
            api_url,
        })
    }

    /// Wait for CVM API to report healthy via /sdk/health
    /// If instance_id is empty, polls VMM to get it and updates the URL
    pub async fn wait_for_cvm_ready(
        &self,
        cvm_api_url: &mut String,
        cvm_id: &str,
        timeout_secs: u64,
    ) -> Result<()> {
        info!(
            cvm_api_url = cvm_api_url,
            timeout = timeout_secs,
            "Waiting for CVM to be ready"
        );

        // Check if instance_id is empty (URL starts with - after https://)
        // Pattern: https://-10000.domain:port vs https://abc123-10000.domain:port
        let has_empty_instance_id = cvm_api_url.contains("https://-")
            || cvm_api_url.contains("wss://-")
            || cvm_api_url.contains("ws://-");

        if has_empty_instance_id {
            // Poll VMM for instance_id (like validator does)
            info!("Instance ID is empty, polling VMM for instance_id...");
            let start = std::time::Instant::now();
            let timeout = std::time::Duration::from_secs(60); // 60s timeout for instance_id
            let mut instance_id = None;

            while instance_id.is_none() && start.elapsed() < timeout {
                match self.get_instance_id(cvm_id).await {
                    Ok(Some(id)) if !id.trim().is_empty() => {
                        instance_id = Some(id);
                        break;
                    }
                    _ => {
                        info!("Waiting for CVM {} instance_id...", cvm_id);
                    }
                }

                // Wait 1 second before retrying
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }

            if let Some(id) = instance_id {
                // Update URL with instance_id
                let (base_domain, gateway_port) = self.get_gateway_meta().await?;
                let host = format!("{}-10000.{}:{}", id, base_domain, gateway_port);
                *cvm_api_url = format!("https://{}", host);
                info!("Updated CVM URL with instance_id: {}", cvm_api_url);
            } else {
                warn!(
                    "Instance ID not available after 60s timeout for CVM {}",
                    cvm_id
                );
            }
        }

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(3))
            .build()?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        while start.elapsed() < timeout {
            let url = format!("{}/sdk/health", cvm_api_url.trim_end_matches('/'));
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    info!(cvm_api_url = cvm_api_url, "CVM is ready");
                    return Ok(());
                }
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        Err(anyhow::anyhow!(
            "Timeout waiting for CVM at {} to be ready",
            cvm_api_url
        ))
    }

    /// Check if CVM is healthy by probing /sdk/health via gateway URL built from instance_id
    pub async fn check_cvm_health(&self, instance_id: &str) -> Result<bool> {
        let (base_domain, gateway_port) = self.get_gateway_meta().await?;
        let host = format!("{}-10000.{}:{}", instance_id, base_domain, gateway_port);
        let api_url = format!("https://{}", host);

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(3))
            .build()?;
        let url = format!("{}/sdk/health", api_url);
        let resp = client.get(&url).send().await;
        Ok(matches!(resp, Ok(r) if r.status().is_success()))
    }

    /// Send credentials to CVM
    pub async fn send_credentials_to_cvm(
        &self,
        cvm_api_url: &str,
        credentials: HashMap<String, String>,
    ) -> Result<()> {
        info!(cvm_api_url = cvm_api_url, "Sending credentials to CVM");

        let response = self
            .http_client
            .post(format!(
                "{}/sdk/admin/db/credentials",
                cvm_api_url.trim_end_matches('/')
            ))
            .json(&CredentialsPayload {
                credentials,
                mode: "api".to_string(),
            })
            .send()
            .await
            .context("Failed to send credentials to CVM")?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "Failed to send credentials: {}",
                error_text
            ));
        }

        Ok(())
    }

    /// Get migrations from CVM
    pub async fn get_migrations_from_cvm(&self, cvm_api_url: &str) -> Result<Vec<Migration>> {
        info!(cvm_api_url = cvm_api_url, "Requesting migrations from CVM");

        let response = self
            .http_client
            .get(format!(
                "{}/sdk/admin/migrations",
                cvm_api_url.trim_end_matches('/')
            ))
            .send()
            .await
            .context("Failed to get migrations from CVM")?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Failed to get migrations: {}", error_text));
        }

        let migrations: Vec<Migration> = response.json().await?;
        Ok(migrations)
    }

    /// Stop a CVM
    pub async fn stop_cvm(&self, instance_id: &str) -> Result<()> {
        info!(instance_id = instance_id, "Stopping CVM");

        let response = self
            .http_client
            .post(format!("{}/prpc/RemoveVm?json", self.vmm_url))
            .json(&serde_json::json!({"id": instance_id}))
            .send()
            .await
            .context("Failed to stop CVM")?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Failed to stop CVM: {}", error_text));
        }

        Ok(())
    }
}

/// Credentials Payload
#[derive(Debug, Serialize, Deserialize)]
struct CredentialsPayload {
    credentials: HashMap<String, String>,
    mode: String,
}

/// VMM metadata (subset)
#[derive(Debug, Deserialize)]
struct VmmMetadata {
    gateway: Option<GatewaySettings>,
}

#[derive(Debug, Deserialize)]
struct GatewaySettings {
    base_domain: String,
    port: u32,
}

/// VMM GetInfo response (subset)
#[derive(Debug, Deserialize)]
struct GetInfoResponse {
    found: bool,
    info: Option<VmInfo>,
}

#[derive(Debug, Deserialize)]
struct VmInfo {
    id: String,
    name: String,
    instance_id: Option<String>,
}

impl CvmManager {
    /// Check if a CVM with the given compose_hash name exists
    async fn check_cvm_exists(&self, compose_hash: &str) -> Result<Option<String>> {
        if self.mock_mode {
            // In mock mode, check if Docker service exists by trying to connect to it
            // For term-challenge, the service is already running as term-challenge-dev
            // compose_hash might be "term-challenge-dev-001" or cvm_name might be "api-term-challenge-..."
            if compose_hash.contains("term-challenge")
                || compose_hash.contains("api-term-challenge")
            {
                // Check if we can connect to the service
                let test_url = "http://term-challenge-dev:10000/sdk/health";
                if let Ok(resp) = self.http_client.get(test_url).send().await {
                    if resp.status().is_success() {
                        return Ok(Some("term-challenge-dev-docker".to_string()));
                    }
                }
            }
            return Ok(None);
        }

        // List all VMs to find one with matching name
        let url = format!("{}/prpc/Status?json", self.vmm_url);
        let resp = self
            .http_client
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await
            .context("Failed to list VMs")?;

        #[derive(Deserialize)]
        struct VmmStatus {
            vms: Vec<VmInfo>,
        }
        let status: VmmStatus = resp.json().await?;

        // Find VM with matching name
        for vm in status.vms {
            if vm.name == compose_hash {
                return Ok(Some(vm.id));
            }
        }

        Ok(None)
    }

    async fn get_gateway_meta(&self) -> Result<(String, u32)> {
        if self.mock_mode {
            // In mock mode, return Docker service URL
            return Ok(("term-challenge-dev".to_string(), 10000));
        }

        let url = format!("{}/prpc/GetMeta?json", self.vmm_url);
        let resp = self
            .http_client
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await
            .context("Failed to query VMM metadata")?;
        let meta: VmmMetadata = resp.json().await?;
        let gw = meta
            .gateway
            .ok_or_else(|| anyhow::anyhow!("No gateway configuration in VMM metadata"))?;
        Ok((gw.base_domain, gw.port))
    }

    async fn get_instance_id(&self, vm_id: &str) -> Result<Option<String>> {
        let url = format!("{}/prpc/GetInfo?json", self.vmm_url);
        let resp = self
            .http_client
            .post(&url)
            .json(&serde_json::json!({ "id": vm_id }))
            .send()
            .await
            .context("Failed to query VMM GetInfo")?;
        let info: GetInfoResponse = resp.json().await?;
        Ok(info.info.and_then(|i| i.instance_id))
    }
}
