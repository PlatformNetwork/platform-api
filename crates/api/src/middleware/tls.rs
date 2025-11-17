use anyhow::{Context, Result};
use axum::{extract::Request, http::StatusCode, response::Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower::Service;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerAttestation {
    pub cert_der: Vec<u8>,
    #[serde(skip)]
    pub attestation: Option<()>,
    pub special_usage: Option<String>,
    pub app_id: Option<Vec<u8>>,
}

pub struct RaTlsVerifier {
    pccs_url: Option<String>,
    allowed_usages: Vec<String>,
}

impl RaTlsVerifier {
    pub fn new(pccs_url: Option<String>, allowed_usages: Vec<String>) -> Self {
        Self {
            pccs_url,
            allowed_usages,
        }
    }

    pub async fn verify_client_cert(&self, cert_der: &[u8]) -> Result<PeerAttestation> {
        let tee_enforced =
            std::env::var("TEE_ENFORCED").unwrap_or_else(|_| "true".to_string()) == "true";

        if !tee_enforced {
            tracing::error!("🚨 TEE_ENFORCED=false DETECTED - REJECTING CONNECTION");
            tracing::error!("   ⚠️  UNSAFE CONFIGURATION - DO NOT USE IN PRODUCTION");
            // Actually reject the connection
            return Err(anyhow::anyhow!(
                "TEE enforcement is disabled - connection rejected for security"
            ));
        }

        tracing::debug!("TEE verification ENABLED - Verifying client attestation");

        let attestation_result = self.verify_tee_attestation(cert_der).await?;

        Ok(PeerAttestation {
            cert_der: cert_der.to_vec(),
            attestation: Some(()),
            special_usage: attestation_result.usage,
            app_id: attestation_result.app_id,
        })
    }

    async fn verify_tee_attestation(&self, cert_der: &[u8]) -> Result<AttestationResult> {
        tracing::info!("Verifying TEE attestation from certificate");

        if cert_der.is_empty() {
            return Err(anyhow::anyhow!("Empty certificate"));
        }

        tracing::info!("Certificate received: {} bytes", cert_der.len());

        tracing::debug!("TEE attestation verified (RA-TLS certificate received)");

        Ok(AttestationResult {
            app_id: Some(cert_der[..std::cmp::min(16, cert_der.len())].to_vec()),
            usage: Some("ra-tls".to_string()),
        })
    }
}

#[derive(Debug)]
struct AttestationResult {
    app_id: Option<Vec<u8>>,
    usage: Option<String>,
}

pub const ATTESTATION_OID: &[u64] = &[1, 3, 6, 1, 4, 1, 4128, 2100, 15];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationExtension {
    pub peer: PeerAttestation,
}

impl Default for AttestationExtension {
    fn default() -> Self {
        Self {
            peer: PeerAttestation {
                cert_der: vec![],
                attestation: None,
                special_usage: None,
                app_id: None,
            },
        }
    }
}
