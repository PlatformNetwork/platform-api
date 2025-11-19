//! CVM credentials management and delivery

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

/// Send credentials to CVM
pub async fn send_credentials_to_cvm(
    gateway_url: &str,
    instance_id: &str,
    compose_hash: &str,
) -> Result<()> {
    info!(
        instance_id = instance_id,
        "Sending credentials to CVM"
    );

    // Generate credentials
    let credentials = generate_credentials(compose_hash)?;

    // Prepare payload
    let payload = CredentialsPayload {
        api_secret: credentials.api_secret,
        compose_hash: compose_hash.to_string(),
        permissions: vec![
            "read".to_string(),
            "write".to_string(),
            "execute".to_string(),
        ],
        expires_at: chrono::Utc::now() + chrono::Duration::hours(24),
    };

    // Send credentials
    let credentials_url = format!("{}/instances/{}/credentials", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .post(&credentials_url)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .context("Failed to send credentials")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to send credentials: {}", error_text));
    }

    info!(
        instance_id = instance_id,
        "Credentials sent successfully"
    );

    Ok(())
}

/// Generate credentials for CVM
fn generate_credentials(compose_hash: &str) -> Result<CvmCredentials> {
    debug!(compose_hash = compose_hash, "Generating CVM credentials");

    use rand::Rng;
    let mut rng = rand::thread_rng();

    // Generate API secret (32 bytes)
    let api_secret: String = (0..32)
        .map(|_| {
            let idx = rng.gen_range(0..char::len_utf8('🦀'));
            char::from_u32(idx.try_into().unwrap()).unwrap_or('a')
        })
        .collect();

    // Generate session token
    let session_token = format!("{}_{}", compose_hash, chrono::Utc::now().timestamp());

    Ok(CvmCredentials {
        api_secret,
        session_token,
        compose_hash: compose_hash.to_string(),
    })
}

/// Rotate credentials for CVM
pub async fn rotate_credentials(
    gateway_url: &str,
    instance_id: &str,
    compose_hash: &str,
) -> Result<()> {
    info!(
        instance_id = instance_id,
        "Rotating CVM credentials"
    );

    // Generate new credentials
    let credentials = generate_credentials(compose_hash)?;

    // Prepare rotation payload
    let payload = CredentialsRotationPayload {
        new_api_secret: credentials.api_secret,
        new_session_token: credentials.session_token,
        reason: "scheduled_rotation".to_string(),
    };

    // Send rotation request
    let rotation_url = format!("{}/instances/{}/credentials/rotate", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .post(&rotation_url)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .context("Failed to rotate credentials")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to rotate credentials: {}", error_text));
    }

    info!(
        instance_id = instance_id,
        "Credentials rotated successfully"
    );

    Ok(())
}

/// Validate credentials
pub async fn validate_credentials(
    gateway_url: &str,
    instance_id: &str,
    api_secret: &str,
) -> Result<bool> {
    debug!(
        instance_id = instance_id,
        "Validating CVM credentials"
    );

    let validation_url = format!("{}/instances/{}/credentials/validate", gateway_url, instance_id);

    #[derive(Serialize)]
    struct ValidationRequest {
        api_secret: String,
    }

    let validation_request = ValidationRequest {
        api_secret: api_secret.to_string(),
    };

    let client = reqwest::Client::new();
    let response = client
        .post(&validation_url)
        .header("Content-Type", "application/json")
        .json(&validation_request)
        .send()
        .await
        .context("Failed to validate credentials")?;

    match response.status() {
        reqwest::StatusCode::OK => Ok(true),
        reqwest::StatusCode::UNAUTHORIZED => Ok(false),
        _ => {
            let error_text = response.text().await
                .context("Failed to read error response")?;
            Err(anyhow::anyhow!("Credential validation failed: {}", error_text))
        }
    }
}

/// Get credential status
pub async fn get_credential_status(
    gateway_url: &str,
    instance_id: &str,
) -> Result<CredentialStatus> {
    debug!(
        instance_id = instance_id,
        "Getting credential status"
    );

    let status_url = format!("{}/instances/{}/credentials/status", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .get(&status_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get credential status")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get credential status: {}", error_text));
    }

    let status: CredentialStatus = response.json().await
        .context("Failed to parse credential status")?;

    debug!(
        instance_id = instance_id,
        status = %status.status,
        expires_at = ?status.expires_at,
        "Retrieved credential status"
    );

    Ok(status)
}

/// Revoke credentials
pub async fn revoke_credentials(
    gateway_url: &str,
    instance_id: &str,
    reason: &str,
) -> Result<()> {
    info!(
        instance_id = instance_id,
        reason = reason,
        "Revoking CVM credentials"
    );

    #[derive(Serialize)]
    struct RevokeRequest {
        reason: String,
    }

    let revoke_request = RevokeRequest {
        reason: reason.to_string(),
    };

    let revoke_url = format!("{}/instances/{}/credentials/revoke", gateway_url, instance_id);

    let client = reqwest::Client::new();
    let response = client
        .post(&revoke_url)
        .header("Content-Type", "application/json")
        .json(&revoke_request)
        .send()
        .await
        .context("Failed to revoke credentials")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to revoke credentials: {}", error_text));
    }

    info!(
        instance_id = instance_id,
        "Credentials revoked successfully"
    );

    Ok(())
}

/// Credentials payload
#[derive(Debug, Serialize)]
struct CredentialsPayload {
    api_secret: String,
    compose_hash: String,
    permissions: Vec<String>,
    expires_at: chrono::DateTime<chrono::Utc>,
}

/// Credentials rotation payload
#[derive(Debug, Serialize)]
struct CredentialsRotationPayload {
    new_api_secret: String,
    new_session_token: String,
    reason: String,
}

/// CVM credentials
#[derive(Debug, Clone)]
struct CvmCredentials {
    api_secret: String,
    session_token: String,
    compose_hash: String,
}

/// Credential status
#[derive(Debug, Deserialize)]
pub struct CredentialStatus {
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub last_used: Option<chrono::DateTime<chrono::Utc>>,
    pub permissions: Vec<String>,
    pub rotation_scheduled: Option<chrono::DateTime<chrono::Utc>>,
}

/// Credential audit log entry
#[derive(Debug, Deserialize)]
pub struct CredentialAuditLog {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub action: String,
    pub instance_id: String,
    pub reason: Option<String>,
    pub success: bool,
}

/// Get credential audit log
pub async fn get_credential_audit_log(
    gateway_url: &str,
    instance_id: &str,
    limit: Option<u32>,
) -> Result<Vec<CredentialAuditLog>> {
    debug!(
        instance_id = instance_id,
        limit = ?limit,
        "Getting credential audit log"
    );

    let audit_url = if let Some(limit) = limit {
        format!("{}/instances/{}/credentials/audit?limit={}", gateway_url, instance_id, limit)
    } else {
        format!("{}/instances/{}/credentials/audit", gateway_url, instance_id)
    };

    let client = reqwest::Client::new();
    let response = client
        .get(&audit_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get audit log")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get audit log: {}", error_text));
    }

    let audit_log: Vec<CredentialAuditLog> = response.json().await
        .context("Failed to parse audit log")?;

    debug!(
        instance_id = instance_id,
        entry_count = audit_log.len(),
        "Retrieved credential audit log"
    );

    Ok(audit_log)
}
