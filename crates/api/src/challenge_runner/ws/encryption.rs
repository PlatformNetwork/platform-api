//! Encryption and decryption utilities

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit};
use rand::RngCore;
use serde_json::Value;

use super::types::EncryptedEnvelope;

/// Encrypt a JSON message using ChaCha20Poly1305
pub fn encrypt_message(msg: &Value, aead_key: &[u8; 32]) -> Result<EncryptedEnvelope> {
    let cipher = ChaCha20Poly1305::new_from_slice(aead_key)
        .map_err(|_| anyhow!("Invalid AEAD key length (expected 32 bytes)"))?;
    
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let nonce_b64 = base64_engine.encode(nonce);

    let msg_bytes = serde_json::to_vec(msg)
        .map_err(|_| anyhow!("Failed to serialize message"))?;

    let ciphertext = cipher
        .encrypt(&nonce.into(), msg_bytes.as_slice())
        .map_err(|_| anyhow!("Failed to encrypt message"))?;
    let ciphertext_b64 = base64_engine.encode(ciphertext);

    Ok(EncryptedEnvelope {
        enc: "chacha20poly1305".to_string(),
        nonce: nonce_b64,
        ciphertext: ciphertext_b64,
    })
}

/// Decrypt an encrypted envelope
pub fn decrypt_envelope(
    envelope: &EncryptedEnvelope,
    aead_key: &[u8; 32],
) -> Result<Value> {
    let nonce_bytes = base64_engine
        .decode(&envelope.nonce)
        .map_err(|e| anyhow!("Failed to decode nonce: {}", e))?;
    
    if nonce_bytes.len() != 12 {
        return Err(anyhow!("Invalid nonce length: {} (expected 12)", nonce_bytes.len()));
    }
    
    let nonce_array: [u8; 12] = nonce_bytes[..12]
        .try_into()
        .map_err(|_| anyhow!("Failed to convert nonce to array"))?;

    let ciphertext = base64_engine
        .decode(&envelope.ciphertext)
        .map_err(|e| anyhow!("Failed to decode ciphertext: {}", e))?;
    
    let cipher = ChaCha20Poly1305::new_from_slice(aead_key)
        .map_err(|_| anyhow!("Invalid AEAD key length"))?;
    
    let plaintext = cipher
        .decrypt(&nonce_array.into(), ciphertext.as_slice())
        .map_err(|e| anyhow!("Decryption failed: {}", e))?;

    serde_json::from_slice(&plaintext)
        .map_err(|e| anyhow!("Failed to parse plain message: {}", e))
}
