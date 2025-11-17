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
    pub updated_at: DateTime<Utc>,
}

