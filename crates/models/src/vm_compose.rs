use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// VM Compose Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmComposeConfig {
    pub id: Uuid,
    pub vm_type: String,
    pub compose_content: String,
    pub description: Option<String>,
    pub required_env: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to update VM compose configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateVmComposeRequest {
    pub compose_content: String,
    pub description: Option<String>,
}

/// Response containing VM compose configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct VmComposeResponse {
    pub vm_type: String,
    pub compose_content: String,
    pub description: Option<String>,
    pub required_env: Vec<String>,
    #[serde(default)]
    pub provisioning: VmProvisioningBundle,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmProvisioningBundle {
    #[serde(default)]
    pub env_keys: Vec<String>,
    pub manifest_defaults: VmManifestDefaults,
    pub vm_parameters: VmHardwareSpec,
}

impl Default for VmProvisioningBundle {
    fn default() -> Self {
        Self {
            env_keys: Vec::new(),
            manifest_defaults: VmManifestDefaults::default(),
            vm_parameters: VmHardwareSpec::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmManifestDefaults {
    pub manifest_version: u32,
    pub name: Option<String>,
    pub runner: String,
    pub kms_enabled: bool,
    pub gateway_enabled: bool,
    pub local_key_provider_enabled: bool,
    pub key_provider_id: String,
    pub public_logs: bool,
    pub public_sysinfo: bool,
    pub public_tcbinfo: bool,
    pub no_instance_id: bool,
    pub secure_time: bool,
}

impl Default for VmManifestDefaults {
    fn default() -> Self {
        Self {
            manifest_version: 2,
            name: Some("validator_vm".to_string()),
            runner: "docker-compose".to_string(),
            kms_enabled: true,
            gateway_enabled: true,
            local_key_provider_enabled: false,
            key_provider_id: String::new(),
            public_logs: true,
            public_sysinfo: true,
            public_tcbinfo: true,
            no_instance_id: false,
            secure_time: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmHardwareSpec {
    pub name: Option<String>,
    pub image: String,
    pub vcpu: u32,
    pub memory: u32,
    pub disk_size: u32,
    #[serde(default)]
    pub user_config: String,
    #[serde(default)]
    pub ports: Vec<VmPortMapping>,
    pub hugepages: bool,
    pub pin_numa: bool,
    pub stopped: bool,
}

impl Default for VmHardwareSpec {
    fn default() -> Self {
        Self {
            name: Some("validator_vm".to_string()),
            image: "dstack-0.5.2".to_string(),
            vcpu: 16,
            memory: 16384,
            disk_size: 200,
            user_config: String::new(),
            ports: Vec::new(),
            hugepages: false,
            pin_numa: false,
            stopped: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmPortMapping {
    pub protocol: String,
    pub host_port: u16,
    pub vm_port: u16,
    pub host_address: Option<String>,
}
