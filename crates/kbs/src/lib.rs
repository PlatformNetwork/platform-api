use anyhow::{Context, Result};
use platform_api_models::{KeyReleaseRequest, KeyReleaseResponse};
use ring::aead::{Aad, BoundKey, Nonce, NonceSequence, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Key Broker Service with envelope encryption
pub struct KeyBrokerService {
    config: KbsConfig,
    random: SystemRandom,
    sessions: Arc<RwLock<HashMap<String, SessionInfo>>>,
}

struct SessionInfo {
    key_id: String,
    key_material: Vec<u8>,
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl KeyBrokerService {
    pub fn new(config: &KbsConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            random: SystemRandom::new(),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn release_key(&self, request: KeyReleaseRequest) -> Result<KeyReleaseResponse> {
        tracing::info!("Releasing key for session: {}", request.session_token);

        // Generate encryption key
        let mut key_bytes = vec![0u8; 32];
        self.random
            .fill(&mut key_bytes)
            .context("Failed to generate random key")?;

        let session_token = request.session_token.clone();
        let expires_at =
            chrono::Utc::now() + chrono::Duration::seconds(self.config.session_timeout as i64);

        // Store session
        let session_info = SessionInfo {
            key_id: uuid::Uuid::new_v4().to_string(),
            key_material: key_bytes.clone(),
            expires_at,
        };

        self.sessions
            .write()
            .await
            .insert(session_token.clone(), session_info);

        Ok(KeyReleaseResponse {
            sealed_key: key_bytes,
            key_id: uuid::Uuid::new_v4().to_string(),
            expires_at,
            policy: "default".to_string(),
            error: None,
        })
    }

    pub async fn verify_key(&self, request: VerifyKeyRequest) -> Result<VerifyKeyResponse> {
        let sessions = self.sessions.read().await;

        if let Some(session) = sessions.get(&request.session_token) {
            if session.expires_at > chrono::Utc::now() {
                return Ok(VerifyKeyResponse {
                    is_valid: true,
                    key_info: KeyInfo {
                        key_id: session.key_id.clone(),
                        algorithm: "AES-256-GCM".to_string(),
                        key_size: 256,
                        created_at: chrono::Utc::now(),
                        expires_at: session.expires_at,
                        usage_count: 0,
                        max_usage: None,
                    },
                    error: None,
                });
            }
        }

        Ok(VerifyKeyResponse {
            is_valid: false,
            key_info: KeyInfo {
                key_id: "".to_string(),
                algorithm: "".to_string(),
                key_size: 0,
                created_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now(),
                usage_count: 0,
                max_usage: None,
            },
            error: Some("Session expired or invalid".to_string()),
        })
    }
}

#[derive(Debug, Clone)]
pub struct KbsConfig {
    pub key_derivation_algorithm: String,
    pub key_size: u32,
    pub session_timeout: u64,
    pub max_sessions: u32,
    pub encryption_key: String,
}

impl Default for KbsConfig {
    fn default() -> Self {
        Self {
            key_derivation_algorithm: "HKDF-SHA256".to_string(),
            key_size: 256,
            session_timeout: 300,
            max_sessions: 1000,
            encryption_key: "change-me-in-production".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct VerifyKeyRequest {
    pub key_id: String,
    pub session_token: String,
    pub policy: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyKeyResponse {
    pub is_valid: bool,
    pub key_info: KeyInfo,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct KeyInfo {
    pub key_id: String,
    pub algorithm: String,
    pub key_size: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub usage_count: u32,
    pub max_usage: Option<u32>,
}
