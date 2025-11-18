use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub backend_type: String,
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub minio_endpoint: Option<String>,
    pub encryption_key: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend_type: "memory".to_string(),
            s3_bucket: None,
            s3_region: None,
            minio_endpoint: None,
            encryption_key: "disabled".to_string(),
        }
    }
}
