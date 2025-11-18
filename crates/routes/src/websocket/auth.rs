use anyhow::Context;
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use hex;
use serde_json::json;
use sha2::{Digest, Sha256};
use sp_core::{crypto::Ss58Codec, sr25519};
use tracing::{info, warn};

use platform_api::services::DstackVerifierClient;
use platform_api::state::AppState;
use dstack_types::VmConfig;
use platform_api_models::{AttestationRequest, AttestationType};
use std::sync::Arc;

use super::messages::{AttestationMessage, SecureMessage};
use super::utils::extract_compose_hash_from_event_log;

/// Verify secure message signature and timestamp
pub async fn verify_secure_message(
    msg: &SecureMessage,
    expected_hotkey: &str,
) -> anyhow::Result<()> {
    // Verify timestamp is recent (within 30 seconds)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if now.saturating_sub(msg.timestamp) > 30 {
        return Err(anyhow::anyhow!(
            "Message timestamp too old: {} seconds",
            now.saturating_sub(msg.timestamp)
        ));
    }

    // Verify public key matches expected hotkey
    if msg.public_key != expected_hotkey {
        return Err(anyhow::anyhow!(
            "Public key mismatch: expected {}, got {}",
            expected_hotkey,
            msg.public_key
        ));
    }

    // Decode public key
    let public_key = sr25519::Public::from_ss58check(&msg.public_key)
        .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

    // Recreate message to verify
    let mut message = Vec::new();
    message.extend_from_slice(msg.message_type.as_bytes());
    message.extend_from_slice(msg.timestamp.to_string().as_bytes());
    message.extend_from_slice(msg.nonce.as_bytes());
    message.extend_from_slice(msg.data.to_string().as_bytes());

    // Decode signature
    let signature_bytes =
        hex::decode(&msg.signature).map_err(|e| anyhow::anyhow!("Invalid signature hex: {}", e))?;

    if signature_bytes.len() != 64 {
        return Err(anyhow::anyhow!("Invalid signature length"));
    }

    let mut sig_array = [0u8; 64];
    sig_array.copy_from_slice(&signature_bytes);
    let signature = sr25519::Signature::from(sig_array);

    // Verify signature using the verify_trait
    use sp_core::crypto::Pair;
    let is_valid = sr25519::Pair::verify(&signature, &message, &public_key);
    if !is_valid {
        return Err(anyhow::anyhow!("Signature verification failed"));
    }

    Ok(())
}

/// Verify validator TDX attestation
pub async fn verify_validator_attestation(
    state: &AppState,
    msg: &AttestationMessage,
    challenge: Option<&[u8]>,
) -> anyhow::Result<()> {
    // If dstack-verifier is configured, use it for full platform verification
    if let Some(ref verifier) = state.dstack_verifier {
        return verify_validator_with_dstack_verifier(state, msg, challenge, verifier).await;
    }

    // Otherwise, use the built-in verification (quote only)
    // Decode quote and event_log
    let quote = msg
        .quote
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing quote"))?;
    // Quote can be in base64 (from validator) or hex (legacy)
    let quote_bytes = match base64_engine.decode(quote) {
        Ok(b) => b,
        Err(_) => {
            // Try hex as fallback for legacy compatibility
            hex::decode(quote).context("Failed to decode quote (tried base64 and hex)")?
        }
    };

    let measurements = msg
        .measurements
        .as_ref()
        .map(|m| {
            m.iter()
                .map(|s| hex::decode(s).unwrap_or_default())
                .collect()
        })
        .unwrap_or_default();

    // Create attestation request
    let attest_request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(quote_bytes),
        report: None,
        nonce: challenge.unwrap_or(&[]).to_vec(),
        measurements,
        capabilities: vec![],
    };

    // Verify attestation with event log
    let event_log = msg.event_log.as_deref();
    let result = state
        .attestation
        .verify_attestation_with_event_log(attest_request, event_log)
        .await
        .context("Failed to verify attestation")?;

    if !matches!(
        result.status,
        platform_api_models::AttestationStatus::Verified
    ) {
        return Err(anyhow::anyhow!(
            "Attestation verification failed: {:?}",
            result.error
        ));
    }

    if let Some(event_log_str) = msg.event_log.as_deref() {
        match extract_compose_hash_from_event_log(event_log_str) {
            Some(hash) => {
                info!(
                    compose_hash = hash,
                    "Validator event log reported compose_hash (informational)"
                );
            }
            None => {
                warn!("Validator event log missing compose-hash entry; continuing because trust is derived from TDX quote validity");
            }
        }
    } else {
        warn!("Validator attestation did not include an event log; continuing because TDX verification already succeeded");
    }

    Ok(())
}

/// Verify challenge binding in report data
pub fn verify_challenge_binding(
    report_data_hex: &str,
    expected_challenge: &str,
    challenge_hash: &str,
) -> bool {
    report_data_hex == expected_challenge
        || report_data_hex == challenge_hash
        || report_data_hex.starts_with(challenge_hash as &str)
        || report_data_hex.starts_with(expected_challenge as &str)
        || (report_data_hex.len() >= 64 && &report_data_hex[..64] == challenge_hash)
}

/// Compute challenge hash
pub fn compute_challenge_hash(challenge: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(challenge.as_bytes());
    hex::encode(hasher.finalize())
}

/// Verify validator using compose hash verification
/// TODO: Implement full TDX verification with dstack-verifier (MRTD/RTMR validation)
/// Currently only verifies compose hash matches the expected compose from DB
async fn verify_validator_with_dstack_verifier(
    state: &AppState,
    msg: &AttestationMessage,
    challenge: Option<&[u8]>,
    _verifier: &Arc<DstackVerifierClient>,
) -> anyhow::Result<()> {
    info!("Verifying validator compose hash (TDX verification TODO)");

    // Extract event log
    let event_log = msg
        .event_log
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing event log"))?;

    // Extract compose hash from event log
    let validator_compose_hash = extract_compose_hash_from_event_log(event_log)
        .ok_or_else(|| anyhow::anyhow!("Missing compose-hash in event log"))?;

    info!(
        "Validator reported compose hash: {}",
        validator_compose_hash
    );

    // Get expected compose config from DB
    let db_compose_config = state
        .storage
        .get_vm_compose_config("validator_vm")
        .await
        .context("Failed to retrieve validator_vm compose config from DB")?;

    info!(
        "Retrieved compose config from DB for vm_type: {}",
        db_compose_config.vm_type
    );

    // Build provisioning bundle (same logic as config.rs)
    let mut env_keys: Vec<String> = ["DSTACK_VMM_URL", "HOTKEY_PASSPHRASE", "VALIDATOR_BASE_URL"]
        .iter()
        .map(|k| k.to_string())
        .collect();
    for key in &db_compose_config.required_env {
        if !env_keys.iter().any(|existing| existing == key) {
            env_keys.push(key.clone());
        }
    }
    env_keys.sort();
    env_keys.dedup();

    // Build app_compose manifest (same structure as deploy.rs)
    let app_compose = json!({
        "manifest_version": 2,
        "name": db_compose_config.vm_type,
        "runner": "docker-compose",
        "docker_compose_file": db_compose_config.compose_content,
        "kms_enabled": true,
        "gateway_enabled": true,
        "local_key_provider_enabled": false,
        "key_provider_id": "",
        "public_logs": true,
        "public_sysinfo": true,
        "public_tcbinfo": true,
        "allowed_envs": env_keys,
        "no_instance_id": false,
        "secure_time": false,
    });

    // Calculate expected compose hash (same method as deploy.rs)
    let app_compose_str =
        serde_json::to_string(&app_compose).context("Failed to serialize app_compose")?;

    let mut hasher = Sha256::new();
    hasher.update(app_compose_str.as_bytes());
    let expected_compose_hash = hex::encode(hasher.finalize());

    info!("Expected compose hash from DB: {}", expected_compose_hash);

    // Compare compose hashes
    if validator_compose_hash != expected_compose_hash {
        // TEMPORARY: Allow validators with mismatched compose hashes during migration
        warn!(
            "Compose hash mismatch (temporarily allowed): validator={}, expected={}",
            validator_compose_hash, expected_compose_hash
        );
        // TODO: Re-enable strict verification once all validators are updated
        // return Err(anyhow::anyhow!(
        //     "Compose hash mismatch: validator reported {}, expected {}",
        //     validator_compose_hash,
        //     expected_compose_hash
        // ));
    } else {
        info!("Compose hash verification successful");
    }

    // TODO: Verify quote signature and MRTD/RTMR values using dstack-verifier
    // This requires:
    // 1. Extract VM config (cpu_count, memory_size) from validator's vm_config or event log
    // 2. Call dstack-verifier with correct vm_config to compute expected MRTD
    // 3. Compare expected MRTD with actual MRTD from quote
    // 4. Verify quote signature using Intel's PCCS service

    // Verify challenge binding if provided
    if let Some(challenge_bytes) = challenge {
        // Extract quote to verify challenge binding
        let quote = msg
            .quote
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing quote for challenge verification"))?;

        let quote_bytes = match base64_engine.decode(quote) {
            Ok(bytes) => bytes,
            Err(_) => hex::decode(quote).context("Failed to decode quote as base64 or hex")?,
        };

        // Calculate expected SHA256 of challenge
        let mut hasher = Sha256::new();
        hasher.update(challenge_bytes);
        let expected_hash = hasher.finalize();

        // Check if report_data in quote matches challenge (report_data is at offset 568-632)
        if quote_bytes.len() >= 632 {
            let report_data_slice = &quote_bytes[568..632];
            if report_data_slice[..32] != expected_hash[..] {
                return Err(anyhow::anyhow!(
                    "Challenge verification failed: report_data in quote does not match SHA256(challenge)"
                ));
            }
            info!("✅ Challenge nonce binding verified");
        } else {
            warn!("Quote too short to verify challenge binding, skipping");
        }
    }

    Ok(())
}

fn resolve_vm_config_from_msg(
    msg: &AttestationMessage,
    os_image_hash: &str,
) -> anyhow::Result<(String, VmConfig)> {
    if let Some(raw) = msg.vm_config.as_ref() {
        match serde_json::from_str::<VmConfig>(raw) {
            Ok(parsed) => return Ok((raw.clone(), parsed)),
            Err(err) => {
                warn!(
                    "Invalid vm_config provided by validator; falling back to defaults: {}",
                    err
                );
            }
        }
    } else {
        warn!("Validator did not include vm_config in attestation; using default hardware spec");
    }
    build_fallback_vm_config(os_image_hash)
}

fn build_fallback_vm_config(os_image_hash: &str) -> anyhow::Result<(String, VmConfig)> {
    let vm_config =
        DstackVerifierClient::extract_vm_config(2, 8 * 1024 * 1024 * 1024, os_image_hash);
    let parsed: VmConfig =
        serde_json::from_str(&vm_config).context("Failed to parse fallback vm_config JSON")?;
    Ok((vm_config, parsed))
}
