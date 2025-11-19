//! TDX attestation and cryptographic handshake handling

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use hkdf::Hkdf;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, warn};
use x25519_dalek::{EphemeralSecret, PublicKey};

use super::types::ConnectionState;
use super::tdx::verify_tdx_quote;

/// Perform TDX attestation handshake
pub async fn perform_attestation(
    read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    conn_state: &ConnectionState,
    api_secret: EphemeralSecret,
    write_handle: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>>>,
) -> Result<[u8; 32]> {
    loop {
        let msg = read
            .next()
            .await
            .ok_or_else(|| anyhow!("Connection closed before attestation"))??;

        if let Message::Text(text) = msg {
            let json: Value = serde_json::from_str(&text)?;

            // Check timeout (30 seconds)
            if let ConnectionState::Unverified { started, .. } = conn_state {
                if started.elapsed().as_secs() > 30 {
                    error!("Attestation timeout");
                    return Err(anyhow!("Attestation timeout"));
                }
            }

            // Expect attestation_response
            if let Some(typ) = json.get("type").and_then(|t| t.as_str()) {
                if typ == "attestation_response" {
                    return handle_attestation_response(json, conn_state, api_secret).await;
                }
            }
        }
    }
}

/// Handle the attestation response from challenge
async fn handle_attestation_response(
    json: Value,
    conn_state: &ConnectionState,
    api_secret: EphemeralSecret,
) -> Result<[u8; 32]> {
    // Extract challenge public key
    let chal_pub_b64 = json
        .get("chal_x25519_pub")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing chal_x25519_pub"))?;
    let chal_pub_bytes = base64_engine.decode(chal_pub_b64)?;
    if chal_pub_bytes.len() != 32 {
        return Err(anyhow!("Invalid challenge public key length"));
    }
    let chal_pub_slice: &[u8; 32] = chal_pub_bytes[..32]
        .try_into()
        .map_err(|_| anyhow!("Failed to convert challenge public key"))?;
    let chal_pub = PublicKey::from(*chal_pub_slice);

    // Derive shared secret
    let shared_secret = api_secret.diffie_hellman(&chal_pub);

    // Get nonce from conn_state
    let nonce = match conn_state {
        ConnectionState::Unverified { nonce, .. } => *nonce,
        _ => return Err(anyhow!("Invalid connection state")),
    };

    // Generate HKDF salt
    let mut hkdf_salt_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut hkdf_salt_bytes);
    let hkdf_salt_b64 = base64_engine.encode(hkdf_salt_bytes);

    // Derive AEAD key using HKDF with salt
    let hkdf = Hkdf::<Sha256>::new(Some(&hkdf_salt_bytes), shared_secret.as_bytes());
    let mut key_bytes = [0u8; 32];
    hkdf.expand(b"platform-api-sdk-v1", &mut key_bytes)
        .map_err(|_| anyhow!("HKDF expansion failed"))?;

    // Verify TDX quote if present
    if !is_dev_mode() {
        if let Some(quote_b64) = json.get("quote").and_then(|v| v.as_str()) {
            match verify_tdx_quote(quote_b64, &nonce).await {
                Ok(_) => debug!("TDX quote verified successfully"),
                Err(e) => {
                    error!("TDX quote verification failed: {}", e);
                    return Err(anyhow!("TDX verification failed: {}", e));
                }
            }
        } else {
            warn!("No TDX quote in attestation_response, skipping verification");
        }
    } else {
        debug!("DEV MODE: Skipping TDX quote verification");
    }

    Ok(key_bytes)
}

/// Check if we're in development mode
fn is_dev_mode() -> bool {
    std::env::var("DEV_MODE")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true"
        || std::env::var("TEE_ENFORCED")
            .unwrap_or_else(|_| "true".to_string())
            .to_lowercase()
            == "false"
}
