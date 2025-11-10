use crate::compose_hash;
use anyhow::{Context, Result};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::OnceCell;

pub struct PlatformSecurity {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    compose_hash: String, // Hash of Docker Compose file (attested by TDX)
}

impl PlatformSecurity {
    pub fn new() -> Result<Self> {
        // Try to calculate compose hash from docker-compose.yml file
        // If file is not found, use a default hash (dev mode fallback)
        let compose_hash = match compose_hash::get_compose_hash_with_env_mode() {
            Ok(hash) => {
                tracing::info!("Calculated compose hash from docker-compose.yml: {}", hash);
                hash
            }
            Err(e) => {
                // Use fallback hash instead of failing
                let fallback_hash = format!("dev-fallback-{}", chrono::Utc::now().timestamp());
                tracing::warn!(
                    "Could not calculate compose hash from docker-compose.yml: {}. Using fallback hash: {}",
                    e,
                    fallback_hash
                );
                fallback_hash
            }
        };

        Self::new_with_compose_hash(&compose_hash)
    }

    /// Initialize from TDX attestation or calculated compose hash (call this at startup)
    pub async fn init_from_tdx() -> Result<Self> {
        // Check if we're in a TDX CVM environment
        let tee_enforced =
            std::env::var("TEE_ENFORCED").unwrap_or_else(|_| "false".to_string()) == "true";
        let dev_mode = std::env::var("DEV_MODE").unwrap_or_else(|_| "false".to_string()) == "true";

        let compose_hash = if tee_enforced && !dev_mode {
            // In production with TEE enforced, try to get compose_hash from TDX attestation
            match Self::get_compose_hash_from_dstack().await {
                Ok(hash) => {
                    tracing::info!("Got compose_hash from TDX attestation: {}", hash);
                    hash
                }
                Err(e) => {
                    // Fail fast in production - no fallback allowed
                    return Err(anyhow::anyhow!(
                        "Security error: Cannot get compose_hash from TDX attestation in production mode. Error: {}. Please ensure TEE_ENFORCED=true only when running in TDX CVM.",
                        e
                    ));
                }
            }
        } else {
            // In dev mode or when TEE is not enforced, try to calculate from docker-compose.yml
            // If file is not found, use a default hash (dev mode fallback)
            match compose_hash::get_compose_hash_with_env_mode() {
                Ok(hash) => {
                    tracing::info!("Calculated compose hash from docker-compose.yml: {}", hash);
                    hash
                }
                Err(e) => {
                    // In dev mode, use a fallback hash instead of failing
                    let fallback_hash = format!("dev-fallback-{}", chrono::Utc::now().timestamp());
                    tracing::warn!(
                        "Could not calculate compose hash from docker-compose.yml: {}. Using fallback hash: {}",
                        e,
                        fallback_hash
                    );
                    fallback_hash
                }
            }
        };

        Self::new_with_compose_hash(&compose_hash)
    }

    pub fn new_with_compose_hash(compose_hash: &str) -> Result<Self> {
        // Generate deterministic key pair based on compose_hash
        // This ensures the same Docker image always generates the same key pair
        let seed = Self::derive_seed_from_compose_hash(compose_hash);
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();

        tracing::info!("Generated security keys from compose_hash");
        tracing::info!("   Compose hash: {}", compose_hash);
        tracing::info!("   Public key: {}", hex::encode(verifying_key.to_bytes()));

        Ok(Self {
            signing_key,
            verifying_key,
            compose_hash: compose_hash.to_string(),
        })
    }

    /// Get compose_hash from dstack TDX attestation
    async fn get_compose_hash_from_dstack() -> Result<String> {
        use reqwest::Client;

        // Get guest-agent URL from environment
        let guest_agent_url = std::env::var("GUEST_AGENT_URL")
            .unwrap_or_else(|_| "http://localhost:8090".to_string());

        let client = Client::new();

        // Call GetQuote endpoint
        let url = format!("{}/prpc/GetQuote", guest_agent_url);
        let report_data = b"platform-api-attestation".to_vec();
        let payload = serde_json::json!({
            "report_data": hex::encode(report_data)
        });

        let response = client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("Failed to call dstack guest-agent")?;

        let quote_response: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse quote response")?;

        // Parse event_log to extract compose_hash
        let event_log_str = quote_response["event_log"]
            .as_str()
            .context("No event_log in response")?;

        let event_log: serde_json::Value =
            serde_json::from_str(event_log_str).context("Failed to parse event_log")?;

        // Extract compose_hash from RTMR3
        let compose_hash = event_log["rtmr3"]["compose_hash"]
            .as_str()
            .context("No compose_hash in event_log")?
            .to_string();

        Ok(compose_hash)
    }

    /// Derive a 32-byte seed from compose_hash
    fn derive_seed_from_compose_hash(compose_hash: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"platform-api-security");
        hasher.update(compose_hash.as_bytes());
        let hash = hasher.finalize();

        let mut seed = [0u8; 32];
        seed.copy_from_slice(&hash[..32]);
        seed
    }

    /// Sign a message with the private key
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let signature = self.signing_key.sign(message);
        signature.to_bytes().to_vec()
    }

    /// Get the public key (to be shared with validators)
    pub fn get_public_key(&self) -> Vec<u8> {
        self.verifying_key.to_bytes().to_vec()
    }

    /// Get the compose hash (from TDX attestation)
    pub fn get_compose_hash(&self) -> &str {
        &self.compose_hash
    }

    /// Create a signed response header value
    pub fn create_signed_header(&self, timestamp: i64, nonce: &str) -> String {
        // Create message: timestamp + nonce
        let message = format!("{}:{}", timestamp, nonce);
        let signature = self.sign(message.as_bytes());
        format!("{}:{}", hex::encode(signature), message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_key_generation() {
        // Same commit should generate same key
        let sec1 = PlatformSecurity::new().unwrap();
        let sec2 = PlatformSecurity::new().unwrap();

        assert_eq!(sec1.get_public_key(), sec2.get_public_key());
    }

    #[test]
    fn test_signature_verification() {
        let sec = PlatformSecurity::new().unwrap();
        let pub_key = sec.get_public_key();

        let message = b"test message";
        let signature = sec.sign(message);

        // Verify signature
        let verifying_key = VerifyingKey::from_bytes(&pub_key[..32].try_into().unwrap()).unwrap();
        let sig_bytes: [u8; 64] = signature.try_into().unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);

        assert!(verifying_key.verify(message, &sig).is_ok());
    }
}
