use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::Result;
use sha2::{Digest, Sha256};

/// Encrypt artifact using AES-256-GCM
pub fn encrypt_artifact(data: &[u8], key: &[u8]) -> Result<EncryptedArtifact> {
    let cipher = Aes256Gcm::new_from_slice(key)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce, data)
        .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

    Ok(EncryptedArtifact {
        ciphertext,
        nonce: nonce.to_vec(),
        mac: vec![], // GCM includes MAC in ciphertext
    })
}

/// Decrypt artifact using AES-256-GCM
pub fn decrypt_artifact(encrypted: &EncryptedArtifact, key: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key)?;
    let nonce = Nonce::from_slice(&encrypted.nonce);

    let plaintext = cipher
        .decrypt(nonce, encrypted.ciphertext.as_ref())
        .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))?;

    Ok(plaintext)
}

/// Generate envelope encryption key
pub fn generate_envelope_key() -> Vec<u8> {
    let mut key = [0u8; 32];
    for byte in key.iter_mut() {
        *byte = rand::random();
    }
    key.to_vec()
}

/// Derive key from master key and artifact ID
pub fn derive_key(master_key: &[u8], artifact_id: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(master_key);
    hasher.update(artifact_id.as_bytes());
    hasher.finalize().to_vec()
}

/// Encrypted artifact structure
#[derive(Debug, Clone)]
pub struct EncryptedArtifact {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub mac: Vec<u8>,
}

impl EncryptedArtifact {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut result = Vec::new();
        result.extend_from_slice(&(self.nonce.len() as u32).to_le_bytes());
        result.extend_from_slice(&self.nonce);
        result.extend_from_slice(&(self.ciphertext.len() as u32).to_le_bytes());
        result.extend_from_slice(&self.ciphertext);
        result
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut offset = 0;

        let nonce_len = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        let nonce = data[offset..offset + nonce_len].to_vec();
        offset += nonce_len;

        let ciphertext_len = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        let ciphertext = data[offset..offset + ciphertext_len].to_vec();

        Ok(EncryptedArtifact {
            ciphertext,
            nonce,
            mac: vec![],
        })
    }
}

/// Artifact metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArtifactMetadata {
    pub id: String,
    pub digest: String,
    pub size: u64,
    pub encrypted: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
