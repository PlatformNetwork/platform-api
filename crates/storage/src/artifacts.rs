use crate::encryption::{encrypt_artifact, EncryptedArtifact};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Artifact storage with hash verification
pub struct ArtifactStorage {
    artifacts: Arc<RwLock<HashMap<Uuid, StoredArtifact>>>,
}

struct StoredArtifact {
    encrypted_data: EncryptedArtifact,
    digest: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl Default for ArtifactStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtifactStorage {
    pub fn new() -> Self {
        Self {
            artifacts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Store encrypted artifact with hash
    pub async fn store_artifact(
        &self,
        submission_id: Uuid,
        data: &[u8],
        key: &[u8],
    ) -> Result<String> {
        // Calculate SHA256 hash
        let mut hasher = Sha256::new();
        hasher.update(data);
        let digest = format!("sha256:{}", hex::encode(hasher.finalize()));

        // Encrypt artifact
        let encrypted = encrypt_artifact(data, key).context("Failed to encrypt artifact")?;

        // Store
        let artifact = StoredArtifact {
            encrypted_data: encrypted,
            digest: digest.clone(),
            created_at: chrono::Utc::now(),
        };

        let mut artifacts = self.artifacts.write().await;
        artifacts.insert(submission_id, artifact);

        Ok(digest)
    }

    /// Retrieve artifact and verify hash
    pub async fn get_artifact(&self, submission_id: Uuid) -> Result<(EncryptedArtifact, String)> {
        let artifacts = self.artifacts.read().await;
        let artifact = artifacts
            .get(&submission_id)
            .ok_or_else(|| anyhow::anyhow!("Artifact not found"))?;

        Ok((artifact.encrypted_data.clone(), artifact.digest.clone()))
    }

    /// Get artifact hash for verification
    pub async fn get_artifact_hash(&self, submission_id: Uuid) -> Result<String> {
        let artifacts = self.artifacts.read().await;
        let artifact = artifacts
            .get(&submission_id)
            .ok_or_else(|| anyhow::anyhow!("Artifact not found"))?;

        Ok(artifact.digest.clone())
    }
}
