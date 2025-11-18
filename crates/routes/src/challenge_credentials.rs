use anyhow::Result;
use axum::{extract::State, http::StatusCode, response::Json};
use base64::{engine::general_purpose::STANDARD as base64, Engine};
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use x25519_dalek::PublicKey;

use platform_api::challenge_migrations::{MigrationOrchestrator, MigrationRequest};
use platform_api::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CredentialRequest {
    pub challenge_id: String,
    pub public_key: String, // Base64 encoded X25519 public key
    pub migrations: Option<Vec<Migration>>,
}

#[derive(Debug, Deserialize)]
pub struct Migration {
    pub version: String,
    pub name: String,
    pub sql: String,
    pub checksum: String,
}

#[derive(Debug, Serialize)]
pub struct CredentialResponse {
    pub encrypted_credentials: EncryptedCredentials,
    pub schema_name: String,
}

#[derive(Debug, Serialize)]
pub struct EncryptedCredentials {
    pub encrypted_data: String,       // Base64
    pub ephemeral_public_key: String, // Base64
    pub nonce: String,                // Base64
}

/// Handle credential requests from TDX-verified challenges via validators
pub async fn request_credentials(
    State(state): State<AppState>,
    Json(request): Json<CredentialRequest>,
) -> Result<Json<CredentialResponse>, StatusCode> {
    info!(
        challenge_id = %request.challenge_id,
        "Received credential request"
    );

    // Parse challenge ID as UUID
    let challenge_uuid = uuid::Uuid::parse_str(&request.challenge_id).map_err(|e| {
        error!("Invalid challenge ID format: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    // Verify the challenge exists and is authorized
    let _challenge = state
        .storage
        .get_challenge(challenge_uuid)
        .await
        .map_err(|e| {
            error!("Failed to get challenge: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Generate schema name from challenge ID
    let schema_name = format!("challenge_{}", request.challenge_id.replace('-', "_"));

    // Create migration orchestrator
    let db_pool = state
        .database_pool
        .as_ref()
        .ok_or_else(|| {
            error!("Database pool not available");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .clone();

    let orchestrator = MigrationOrchestrator::new((*db_pool).clone());

    // Create challenge database schema
    orchestrator
        .create_challenge_database(&request.challenge_id, &schema_name)
        .await
        .map_err(|e| {
            error!("Failed to create challenge database: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Apply migrations if provided
    if let Some(migrations) = request.migrations {
        let migration_request = MigrationRequest {
            challenge_id: request.challenge_id.clone(),
            schema_name: schema_name.clone(),
            migrations: migrations
                .into_iter()
                .map(|m| platform_api::challenge_migrations::Migration {
                    version: m.version,
                    name: m.name,
                    sql: m.sql,
                    checksum: m.checksum,
                })
                .collect(),
        };

        orchestrator
            .apply_migrations(migration_request)
            .await
            .map_err(|e| {
                error!("Failed to apply migrations: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    // Generate database credentials
    let credentials = orchestrator
        .generate_challenge_credentials(&request.challenge_id, &schema_name)
        .await
        .map_err(|e| {
            error!("Failed to generate credentials: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Decode the challenge's public key
    let public_key_bytes = base64.decode(&request.public_key).map_err(|e| {
        error!("Invalid public key encoding: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    if public_key_bytes.len() != 32 {
        error!("Invalid public key length: {}", public_key_bytes.len());
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut public_key_array = [0u8; 32];
    public_key_array.copy_from_slice(&public_key_bytes);
    let recipient_public_key = PublicKey::from(public_key_array);

    // Encrypt credentials
    let encrypted = encrypt_credentials(&credentials, &recipient_public_key).map_err(|e| {
        error!("Failed to encrypt credentials: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!(
        challenge_id = %request.challenge_id,
        schema = %schema_name,
        "Successfully generated and encrypted credentials"
    );

    Ok(Json(CredentialResponse {
        encrypted_credentials: encrypted,
        schema_name,
    }))
}

/// Encrypt credentials using X25519 + ChaCha20Poly1305
fn encrypt_credentials(
    credentials: &std::collections::HashMap<String, String>,
    recipient_public_key: &PublicKey,
) -> Result<EncryptedCredentials> {
    use chacha20poly1305::{
        aead::{Aead, KeyInit},
        ChaCha20Poly1305, Nonce,
    };
    use hkdf::Hkdf;
    use rand::{rngs::OsRng, RngCore};
    use sha2::Sha256;

    // Generate ephemeral secret key
    let mut ephemeral_secret_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut ephemeral_secret_bytes);

    // Perform X25519 scalar multiplication to get public key
    let ephemeral_public_bytes = x25519_function(&ephemeral_secret_bytes, &X25519_BASEPOINT_BYTES);
    let ephemeral_public = PublicKey::from(ephemeral_public_bytes);

    // Compute shared secret
    let shared_secret = x25519_function(&ephemeral_secret_bytes, recipient_public_key.as_bytes());

    // Derive encryption key using HKDF
    let hkdf = Hkdf::<Sha256>::new(Some(b"platform-credential-transfer-v1"), &shared_secret);
    let mut okm = [0u8; 32];
    hkdf.expand(b"credential-encryption", &mut okm)
        .map_err(|_| anyhow::anyhow!("HKDF expansion failed"))?;

    // Create cipher
    let cipher = ChaCha20Poly1305::new_from_slice(&okm)
        .map_err(|_| anyhow::anyhow!("Cipher creation failed"))?;

    // Generate nonce
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Serialize credentials
    let plaintext = serde_json::to_vec(credentials)?;

    // Encrypt with associated data
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| anyhow::anyhow!("Encryption failed"))?;

    Ok(EncryptedCredentials {
        encrypted_data: base64.encode(&ciphertext),
        ephemeral_public_key: base64.encode(ephemeral_public.as_bytes()),
        nonce: base64.encode(nonce_bytes),
    })
}

// X25519 basepoint
const X25519_BASEPOINT_BYTES: [u8; 32] = [
    9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

// X25519 scalar multiplication using the reference implementation
fn x25519_function(k: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    use x25519_dalek::x25519;
    x25519(*k, *u)
}

/// Create the challenge credentials router
pub fn create_router() -> axum::Router<AppState> {
    use axum::routing::post;

    axum::Router::new().route("/challenges/:id/credentials", post(request_credentials))
}
