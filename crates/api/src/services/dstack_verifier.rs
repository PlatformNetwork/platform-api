use anyhow::{Context, Result};
use dstack_types::VmConfig;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRequest {
    pub quote: String,
    pub event_log: String,
    pub vm_config: String,
    pub pccs_url: Option<String>,
    pub debug: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResponse {
    pub is_valid: bool,
    pub details: VerificationDetails,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationDetails {
    pub quote_verified: bool,
    pub event_log_verified: bool,
    pub os_image_hash_verified: bool,
    pub report_data: Option<String>,
    pub tcb_status: Option<String>,
    pub advisory_ids: Vec<String>,
    pub app_info: Option<AppInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub compose_hash: Option<String>,
    pub app_id: Option<String>,
    pub instance_id: Option<String>,
    pub key_provider: Option<KeyProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyProvider {
    pub kms_id: String,
    pub kms_name: String,
}

/// Client for dstack-verifier service
#[derive(Clone)]
pub struct DstackVerifierClient {
    client: reqwest::Client,
    base_url: String,
}

impl DstackVerifierClient {
    pub fn new(base_url: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client, base_url })
    }

    /// Verify a TDX quote with full platform verification
    pub async fn verify(&self, request: VerificationRequest) -> Result<VerificationResponse> {
        info!("Sending verification request to dstack-verifier");

        let response = self
            .client
            .post(format!("{}/verify", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to send verification request")?;

        if !response.status().is_success() {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Verification failed: {}", error);
        }

        let result = response
            .json::<VerificationResponse>()
            .await
            .context("Failed to parse verification response")?;

        info!("Verification completed - valid: {}", result.is_valid);

        Ok(result)
    }

    /// Extract VM configuration from validator
    pub fn extract_vm_config(cpu_count: u32, memory_size: u64, os_image_hash: &str) -> String {
        let os_image_hash_bytes = match hex::decode(os_image_hash) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!(
                    "Failed to decode os_image_hash '{}' as hex: {}. Using empty hash.",
                    os_image_hash, err
                );
                Vec::new()
            }
        };

        let vm_config = VmConfig {
            spec_version: 1,
            os_image_hash: os_image_hash_bytes,
            cpu_count,
            memory_size,
            qemu_single_pass_add_pages: Some(false),
            pic: Some(false),
            qemu_version: None,
            pci_hole64_size: 0,
            hugepages: false,
            num_gpus: 0,
            num_nvswitches: 0,
            hotplug_off: true,
            image: None,
        };

        serde_json::to_string(&vm_config).unwrap_or_else(|err| {
            warn!("Failed to serialize vm_config: {}", err);
            "{}".to_string()
        })
    }
}

/// Parse VM configuration from validator data
pub fn parse_validator_vm_config(validator_data: &serde_json::Value) -> Result<String> {
    // Extract VM configuration from validator attestation data
    let cpu_count = validator_data
        .get("cpu_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(2) as u32;

    let memory_size = validator_data
        .get("memory_size")
        .and_then(|v| v.as_u64())
        .unwrap_or(8 * 1024 * 1024 * 1024); // 8GB default

    let os_image_hash = validator_data
        .get("os_image_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing os_image_hash"))?;

    Ok(DstackVerifierClient::extract_vm_config(
        cpu_count,
        memory_size,
        os_image_hash,
    ))
}
