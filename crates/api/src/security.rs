use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};

pub struct PlatformSecurity {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    compose_hash: String, // Hash of Docker Compose file (attested by TDX)
}

impl PlatformSecurity {
    pub fn new() -> Result<Self> {
        // Generate random compose hash (not verified, randomized)
        use rand::RngCore;
        let mut random_bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut random_bytes);
        let compose_hash = format!("random-{}", hex::encode(random_bytes));

        tracing::info!(
            "Generated random compose hash (not verified): {}",
            compose_hash
        );

        Self::new_with_random_keys(&compose_hash)
    }

    /// Initialize from TDX attestation or calculated compose hash (call this at startup)
    /// Now uses random keys (not verified)
    pub async fn init_from_tdx() -> Result<Self> {
        // Generate random compose hash (not verified, randomized)
        use rand::RngCore;
        let mut random_bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut random_bytes);
        let compose_hash = format!("random-{}", hex::encode(random_bytes));

        tracing::info!(
            "Generated random compose hash (not verified): {}",
            compose_hash
        );

        Self::new_with_random_keys(&compose_hash)
    }

    /// Generate random keys (not deterministic, not verified)
    pub fn new_with_random_keys(compose_hash: &str) -> Result<Self> {
        // Generate random key pair (not deterministic)
        use rand::rngs::OsRng;
        use rand::RngCore;

        // Generate random 32-byte secret key
        let mut secret_key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut secret_key_bytes);

        // Create SigningKey directly from bytes (like validator does)
        let signing_key = SigningKey::from_bytes(&secret_key_bytes);
        let verifying_key = signing_key.verifying_key();
        let public_key_bytes = verifying_key.to_bytes();

        // Validate that public key was generated correctly
        if public_key_bytes.is_empty() {
            return Err(anyhow::anyhow!(
                "Failed to generate public key: key is empty"
            ));
        }
        if public_key_bytes.len() != 32 {
            return Err(anyhow::anyhow!(
                "Invalid public key length: expected 32 bytes, got {} bytes",
                public_key_bytes.len()
            ));
        }

        tracing::info!("Generated random security keys (not verified)");
        tracing::info!("   Compose hash: {} (random, not verified)", compose_hash);
        tracing::info!(
            "   Public key: {} ({} bytes)",
            hex::encode(public_key_bytes),
            public_key_bytes.len()
        );

        Ok(Self {
            signing_key,
            verifying_key,
            compose_hash: compose_hash.to_string(),
        })
    }

    /// Legacy method - kept for compatibility but uses random keys
    pub fn new_with_compose_hash(compose_hash: &str) -> Result<Self> {
        Self::new_with_random_keys(compose_hash)
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
        let report_data = b"
        platform-api-attestation"
            .to_vec();
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

    // Removed derive_seed_from_compose_hash - keys are now random, not deterministic

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
    fn test_random_key_generation() {
        // Keys should be random (different each time)
        let sec1 = PlatformSecurity::new().unwrap();
        let sec2 = PlatformSecurity::new().unwrap();

        // Keys should be different (randomized)
        assert_ne!(sec1.get_public_key(), sec2.get_public_key());
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
