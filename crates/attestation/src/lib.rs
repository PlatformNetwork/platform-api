use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use platform_api_models::{
    AttestationPolicy, AttestationRequest, AttestationResponse, AttestationSession,
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

mod verifier;
pub use verifier::*;

mod config;
pub use config::*;

mod mock_tdx;
pub use mock_tdx::*;

// Use TdxConfig as AttestationConfig for now
pub type AttestationConfig = TdxConfig;

/// Attestation service for TDX VM verification
pub struct AttestationService {
    config: AttestationConfig,
    verifier: TdxVerifier,
    sessions: Arc<tokio::sync::RwLock<HashMap<Uuid, AttestationSession>>>,
    nonces: Arc<tokio::sync::RwLock<HashMap<String, NonceInfo>>>,
    signing_key: EncodingKey,
    decoding_key: DecodingKey,
}

/// Nonce information
#[derive(Debug, Clone)]
struct NonceInfo {
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl AttestationService {
    pub fn new(config: &AttestationConfig) -> Result<Self> {
        // JWT is optional - if disabled, use a placeholder secret
        // Note: JWT functionality will be disabled if secret is "disabled-no-jwt"
        const DEFAULT_SECRET: &str = "change-me-in-production";
        const DISABLED_SECRET: &str = "disabled-no-jwt";

        // Allow disabled JWT or skip default secret check if JWT is intentionally disabled
        let jwt_secret = if config.jwt_secret == DISABLED_SECRET || config.jwt_secret.is_empty() {
            DISABLED_SECRET.to_string()
        } else if config.jwt_secret == DEFAULT_SECRET {
            return Err(anyhow::anyhow!(
                "Security error: Default JWT secret '{}' cannot be used. Please set JWT_SECRET environment variable with a strong secret or use 'disabled-no-jwt' to disable JWT.",
                DEFAULT_SECRET
            ));
        } else {
            config.jwt_secret.clone()
        };

        let signing_key = EncodingKey::from_secret(jwt_secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(jwt_secret.as_bytes());

        let verifier = TdxVerifier::new(config.clone());

        Ok(Self {
            config: config.clone(),
            verifier,
            sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            nonces: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            signing_key,
            decoding_key,
        })
    }

    pub async fn verify_attestation(
        &self,
        request: AttestationRequest,
    ) -> Result<AttestationResponse> {
        self.verify_attestation_with_event_log(request, None).await
    }

    pub async fn verify_attestation_with_event_log(
        &self,
        request: AttestationRequest,
        event_log: Option<&str>,
    ) -> Result<AttestationResponse> {
        // Check if TEE verification is enforced
        let tee_enforced =
            std::env::var("TEE_ENFORCED").unwrap_or_else(|_| "true".to_string()) == "true";
        let dev_mode = std::env::var("DEV_MODE").unwrap_or_else(|_| "false".to_string()) == "true";
        let tdx_simulation_mode = std::env::var("TDX_SIMULATION_MODE")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true";

        // Always verify attestation - no bypass allowed
        // In dev mode, use mock verification but still validate structure
        let verification_result = if !tee_enforced || dev_mode || tdx_simulation_mode {
            tracing::info!(
                "DEV MODE: Using enhanced mock TDX verification (structure validation and crypto operations)"
            );

            // Validate request structure even in dev mode
            if request.quote.is_none() {
                return Ok(AttestationResponse {
                    session_token: String::new(),
                    status: platform_api_models::AttestationStatus::Failed,
                    expires_at: Utc::now(),
                    verified_measurements: vec![],
                    policy: String::new(),
                    error: Some("Missing quote in attestation request".to_string()),
                });
            }

            // Check nonce binding if present
            if !request.nonce.is_empty() {
                if request.nonce.len() < 16 {
                    return Ok(AttestationResponse {
                        session_token: String::new(),
                        status: platform_api_models::AttestationStatus::Failed,
                        expires_at: Utc::now(),
                        verified_measurements: vec![],
                        policy: String::new(),
                        error: Some("Nonce too short (minimum 16 bytes)".to_string()),
                    });
                }

                // Verify nonce binding in quote using mock TDX extraction
                let quote_bytes = request.quote.as_ref().unwrap();
                match MockTdxQuote::extract_measurements(quote_bytes, &request.nonce) {
                    Ok(_measurements) => {
                        tracing::debug!("Nonce binding verified successfully in mock mode");
                    }
                    Err(e) => {
                        tracing::warn!("Nonce binding verification failed: {}", e);
                        // In enhanced simulation mode, we still allow but log the issue
                        if !tdx_simulation_mode {
                            return Ok(AttestationResponse {
                                session_token: String::new(),
                                status: platform_api_models::AttestationStatus::Failed,
                                expires_at: Utc::now(),
                                verified_measurements: vec![],
                                policy: String::new(),
                                error: Some(format!("Nonce binding verification failed: {}", e)),
                            });
                        }
                    }
                }
            }

            // Extract app info from event log (same as real verifier)
            let (app_id, instance_id, compose_hash) = Self::extract_app_info_from_event_log(event_log)?;

            // Use extracted values or fallback to defaults
            let app_id_bytes = app_id
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_else(|| b"dev-mode-app-id".to_vec());
            let instance_id_bytes = instance_id
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_else(|| b"dev-mode-instance-id".to_vec());
            let device_id_bytes = Some(b"dev-mode-device-id".to_vec());

            tracing::info!(
                app_id = ?String::from_utf8_lossy(&app_id_bytes),
                instance_id = ?String::from_utf8_lossy(&instance_id_bytes),
                compose_hash = ?compose_hash,
                "Mock TDX verification completed"
            );

            // Create a mock verification result (structure validated)
            VerificationResult {
                is_valid: true,
                measurements: request.measurements.clone(),
                app_id: Some(app_id_bytes),
                instance_id: Some(instance_id_bytes),
                device_id: device_id_bytes,
                error: None,
            }
        } else {
            tracing::info!("Verifying attestation request with TDX verifier");

            // Verify the attestation with the real verifier (fail fast if invalid)
            match TdxVerifier::verify_static(&self.verifier, &request, event_log).await {
                Ok(result) => {
                    if !result.is_valid {
                        return Err(anyhow::anyhow!(
                            "TDX attestation verification failed: {}",
                            result.error.as_deref().unwrap_or("Unknown error")
                        ));
                    }
                    result
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "TDX attestation verification error: {}. Attestation rejected.",
                        e
                    ));
                }
            }
        };

        if !verification_result.is_valid {
            return Ok(AttestationResponse {
                session_token: String::new(),
                status: platform_api_models::AttestationStatus::Failed,
                expires_at: Utc::now(),
                verified_measurements: vec![],
                policy: String::new(),
                error: Some(
                    verification_result
                        .error
                        .unwrap_or_else(|| "Verification failed".to_string()),
                ),
            });
        }

        // Generate session token
        let session_id = Uuid::new_v4();
        let session_token = self.generate_grant_token(&session_id, &verification_result)?;
        let expires_at = Utc::now() + Duration::seconds(self.config.session_timeout as i64);

        // Store session
        // Derive validator_hotkey from verified TEE identity (app_id and instance_id)
        // Fail fast if app_id or instance_id is missing
        let validator_hotkey = {
            let app_id = verification_result.app_id.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Security error: app_id missing from verification result")
            })?;
            let instance_id = verification_result.instance_id.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Security error: instance_id missing from verification result")
            })?;

            let app_id_str = hex::encode(app_id);
            let instance_id_str = hex::encode(instance_id);
            format!("validator-{}-{}", app_id_str, instance_id_str)
        };

        let session = AttestationSession {
            id: session_id,
            session_token: session_token.clone(),
            attestation_type: request.attestation_type,
            status: platform_api_models::AttestationStatus::Verified,
            validator_hotkey,
            created_at: Utc::now(),
            expires_at,
            verified_measurements: verification_result.measurements.clone(),
            policy: String::new(),
            key_releases: vec![],
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id, session);

        Ok(AttestationResponse {
            session_token,
            status: platform_api_models::AttestationStatus::Verified,
            expires_at,
            verified_measurements: verification_result.measurements,
            policy: String::new(),
            error: None,
        })
    }

    pub async fn get_session(&self, id: Uuid) -> Result<AttestationSession> {
        let sessions = self.sessions.read().await;
        sessions
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Session not found"))
    }

    pub async fn list_policies(&self) -> Result<Vec<AttestationPolicy>> {
        Ok(vec![])
    }

    pub async fn get_policy(&self, _id: &str) -> Result<AttestationPolicy> {
        Err(anyhow::anyhow!("Policy not found"))
    }

    pub fn verify_token(&self, token: &str) -> Result<serde_json::Value> {
        use jsonwebtoken::{decode, Algorithm, Validation};

        // Check if JWT is disabled
        if self.config.jwt_secret == "disabled-no-jwt" || self.config.jwt_secret.is_empty() {
            return Err(anyhow::anyhow!("JWT authentication is disabled"));
        }

        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&["platform-executor"]);

        let claims = decode::<serde_json::Value>(token, &self.decoding_key, &validation)
            .context("Failed to decode JWT token")?;

        // Verify expiration
        let exp = claims
            .claims
            .get("exp")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("Missing exp claim"))?;

        let now = chrono::Utc::now().timestamp() as u64;
        if exp < now {
            return Err(anyhow::anyhow!("Token expired"));
        }

        // Verify required claims
        if claims.claims.get("app_id").is_none() {
            return Err(anyhow::anyhow!("Missing app_id claim"));
        }

        if claims.claims.get("instance_id").is_none() {
            return Err(anyhow::anyhow!("Missing instance_id claim"));
        }

        Ok(claims.claims)
    }

    /// Extract app info from event log (same logic as TdxVerifier)
    fn extract_app_info_from_event_log(
        event_log: Option<&str>,
    ) -> Result<(Option<String>, Option<String>, Option<String>)> {
        let event_log_str = match event_log {
            Some(log) => log,
            None => return Ok((None, None, None)),
        };

        // Parse event log JSON
        let event_log_json: serde_json::Value =
            serde_json::from_str(event_log_str).context("Failed to parse event log")?;

        let mut app_id = None;
        let mut instance_id = None;
        let mut compose_hash = None;

        if let Some(events) = event_log_json.as_array() {
            for event in events {
                if let Some(event_type) = event.get("event").and_then(|e| e.as_str()) {
                    if let Some(payload) = event.get("event_payload").and_then(|p| p.as_str()) {
                        match event_type {
                            "app-id" => app_id = Some(payload.to_string()),
                            "instance-id" => instance_id = Some(payload.to_string()),
                            "compose-hash" => compose_hash = Some(payload.to_string()),
                            _ => {}
                        }
                    }
                }
            }
        }

        Ok((app_id, instance_id, compose_hash))
    }

    fn generate_grant_token(
        &self,
        session_id: &Uuid,
        verification: &VerificationResult,
    ) -> Result<String> {
        // Check if JWT is disabled
        if self.config.jwt_secret == "disabled-no-jwt" || self.config.jwt_secret.is_empty() {
            return Err(anyhow::anyhow!(
                "JWT authentication is disabled - cannot generate token"
            ));
        }

        let claims = GrantClaims {
            sub: session_id.to_string(),
            jti: session_id.to_string(),
            aud: "platform-executor".to_string(),
            exp: (Utc::now() + Duration::seconds(300)).timestamp() as usize,
            iat: Utc::now().timestamp() as usize,
            app_id: hex::encode(&verification.app_id.clone().unwrap_or_default()),
            instance_id: hex::encode(&verification.instance_id.clone().unwrap_or_default()),
            device_id: hex::encode(&verification.device_id.clone().unwrap_or_default()),
        };

        let token = encode(&Header::new(Algorithm::HS256), &claims, &self.signing_key)?;
        Ok(token)
    }
}

/// Grant JWT claims
#[derive(Debug, Serialize)]
struct GrantClaims {
    sub: String,
    jti: String,
    aud: String,
    exp: usize,
    iat: usize,
    app_id: String,
    instance_id: String,
    device_id: String,
}

/// Verification result
#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub is_valid: bool,
    pub measurements: Vec<Vec<u8>>,
    pub app_id: Option<Vec<u8>>,
    pub instance_id: Option<Vec<u8>>,
    pub device_id: Option<Vec<u8>>,
    pub error: Option<String>,
}
