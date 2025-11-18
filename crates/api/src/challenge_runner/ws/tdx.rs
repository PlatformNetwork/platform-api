//! TDX quote verification

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use sha2::{Digest, Sha256};

/// Verify TDX quote by checking report_data matches SHA256(nonce)
/// Note: This verifies the challenge binding but does not verify the cryptographic signature
pub async fn verify_tdx_quote(quote_b64: &str, nonce_bytes: &[u8; 32]) -> Result<()> {
    let quote_bytes = base64_engine
        .decode(quote_b64)
        .context("Failed to decode quote from base64")?;

    tracing::info!("Decoded TDX quote: {} bytes", quote_bytes.len());

    // Verify report_data matches SHA256(nonce)
    let mut hasher = Sha256::new();
    hasher.update(nonce_bytes);
    let expected = hasher.finalize();

    // TDX report_data location can vary slightly between quote versions.
    // Try common offsets and accept a match against SHA256(nonce).
    let candidate_offsets: [usize; 3] = [568, 576, 584];
    let mut matched = false;
    let mut matched_off: Option<usize> = None;
    for off in candidate_offsets.iter() {
        if quote_bytes.len() >= off + 32 {
            let rd = &quote_bytes[*off..*off + 32];
            if rd == expected.as_slice() {
                matched = true;
                matched_off = Some(*off);
                break;
            }
        }
    }

    if !matched {
        return Err(anyhow!(
            "report_data mismatch: quote report_data does not match SHA256(nonce)"
        ));
    }

    if let Some(off) = matched_off {
        tracing::info!("✅ Matched report_data at offset {}", off);
    }

    Ok(())
}
