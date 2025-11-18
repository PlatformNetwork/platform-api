use crate::config::TdxConfig;
use crate::VerificationResult;
use anyhow::{Context, Result};
use platform_api_models::AttestationRequest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;

#[derive(Debug)]
pub struct TdxVerifier {
    config: TdxConfig,
    client: reqwest::Client,
}

// Event log structures
#[derive(Serialize, Deserialize, Debug)]
struct EventLogEvent {
    event: String,
    event_payload: String,
}

const DEFAULT_PCCS_URL: &str = "https://pccs.bittensor.com/sgx/certification/v4/";

impl TdxVerifier {
    pub fn new(config: TdxConfig) -> Self {
        let client = reqwest::Client::builder()
            .build()
            .expect("Failed to create HTTP client");

        Self { config, client }
    }

    pub async fn verify_static(
        &self,
        request: &AttestationRequest,
        event_log: Option<&str>,
    ) -> Result<VerificationResult> {
        tracing::info!("Verifying TDX attestation with dcap-qvl");

        let quote = request
            .quote
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing quote"))?;

        // Basic validation: quote should not be empty
        if quote.is_empty() {
            return Ok(VerificationResult {
                is_valid: false,
                measurements: request.measurements.clone(),
                app_id: None,
                instance_id: None,
                device_id: None,
                error: Some("Quote is empty".to_string()),
            });
        }

        tracing::info!("Quote received: {} bytes", quote.len());

        // Verify the quote using dcap-qvl
        let pccs_url = self.config.pccs_url.as_deref().unwrap_or(DEFAULT_PCCS_URL);

        // Get collateral from PCCS or Intel PCS
        let collateral = match dcap_qvl::collateral::get_collateral(pccs_url, quote).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Failed to get collateral from PCCS, trying Intel PCS: {}",
                    e
                );
                // Fallback to Intel PCS
                dcap_qvl::collateral::get_collateral_from_pcs(quote)
                    .await
                    .context("Failed to get collateral from Intel PCS")?
            }
        };

        // Verify the quote
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();

        let verified_report = dcap_qvl::verify::verify(quote, &collateral, now)
            .context("Failed to verify TDX quote")?;

        tracing::info!(
            "TDX quote verified successfully - TCB Status: {}",
            verified_report.status
        );

        // Only accept quotes with valid TCB status
        let valid_statuses = ["UpToDate", "SWHardeningNeeded", "ConfigurationNeeded"];
        let is_valid = valid_statuses.contains(&verified_report.status.as_str());

        if !is_valid {
            return Ok(VerificationResult {
                is_valid: false,
                measurements: request.measurements.clone(),
                app_id: None,
                instance_id: None,
                device_id: None,
                error: Some(format!("Invalid TCB status: {}", verified_report.status)),
            });
        }

        // Verify nonce binding if nonce is provided
        if !request.nonce.is_empty() {
            // Parse the quote to get report data
            let quote_struct = dcap_qvl::quote::Quote::parse(quote)
                .map_err(|e| anyhow::anyhow!("Failed to parse quote: {:?}", e))?;

            // Get report data based on report type
            let report_data = match &quote_struct.report {
                dcap_qvl::quote::Report::SgxEnclave(enclave_report) => &enclave_report.report_data,
                dcap_qvl::quote::Report::TD10(td_report) => &td_report.report_data,
                dcap_qvl::quote::Report::TD15(td_report) => &td_report.base.report_data,
            };

            // Verify nonce binding: SHA256(nonce) must be in first 32 bytes of report_data
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&request.nonce);
            let expected_nonce_hash = hasher.finalize();

            // TDX places SHA256(nonce) in the first 32 bytes of report_data
            if report_data.len() < 32 || &report_data[..32] != &expected_nonce_hash[..] {
                return Ok(VerificationResult {
                    is_valid: false,
                    measurements: request.measurements.clone(),
                    app_id: None,
                    instance_id: None,
                    device_id: None,
                    error: Some("Nonce binding verification failed: report_data does not match SHA256(nonce)".to_string()),
                });
            }
            info!("✅ Nonce binding verified successfully");
        }

        // Extract app info from event log
        let (app_id, instance_id, compose_hash) = self.extract_app_info(event_log)?;

        tracing::info!(
            app_id = ?app_id,
            instance_id = ?instance_id,
            compose_hash = ?compose_hash,
            "Attestation verified successfully with dcap-qvl"
        );

        Ok(VerificationResult {
            is_valid: true,
            measurements: request.measurements.clone(),
            app_id: app_id.map(|s| s.as_bytes().to_vec()),
            instance_id: instance_id.map(|s| s.as_bytes().to_vec()),
            device_id: None,
            error: None,
        })
    }

    fn extract_app_info(
        &self,
        event_log: Option<&str>,
    ) -> Result<(Option<String>, Option<String>, Option<String>)> {
        let event_log = match event_log {
            Some(log) => log,
            None => return Ok((None, None, None)),
        };

        let events: Vec<EventLogEvent> = serde_json::from_str(event_log)?;
        let mut event_map = HashMap::new();
        for event in events {
            event_map.insert(event.event, event.event_payload);
        }

        let app_id = event_map.get("app-id").cloned();
        let instance_id = event_map.get("instance-id").cloned();
        let compose_hash = event_map.get("compose-hash").cloned();

        Ok((app_id, instance_id, compose_hash))
    }

    // Replay RTMR calculation (for future use with TDX RTMR verification)
    fn replay_rtmr(&self, history: &[String]) -> String {
        use sha2::{Digest, Sha256, Sha384};

        let mut rtmr = vec![0u8; 48]; // RTMR is 384 bits

        for entry in history {
            let mut hasher = Sha256::new();
            hasher.update(entry.as_bytes());
            let digest = hasher.finalize();

            let mut combined = rtmr.clone();
            combined.extend_from_slice(&digest);

            let mut hasher384 = Sha384::new();
            hasher384.update(&combined);
            let result = hasher384.finalize();

            rtmr.copy_from_slice(&result[..48]);
        }

        hex::encode(rtmr)
    }

    // Validate event structure (for future use)
    fn validate_event(&self, event: &EventLogEvent) -> bool {
        match event.event.as_str() {
            "app-id" | "instance-id" | "compose-hash" => !event.event_payload.is_empty(),
            _ => true, // Unknown events are allowed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replay_rtmr() {
        let config = TdxConfig::from_env();
        let verifier = TdxVerifier::new(config);

        let history = vec!["event1".to_string(), "event2".to_string()];
        let rtmr = verifier.replay_rtmr(&history);

        assert_eq!(rtmr.len(), 96); // 48 bytes = 96 hex chars
    }

    #[test]
    fn test_validate_event() {
        let config = TdxConfig::from_env();
        let verifier = TdxVerifier::new(config);

        let event = EventLogEvent {
            event: "app-id".to_string(),
            event_payload: "test-app".to_string(),
        };

        assert!(verifier.validate_event(&event));

        let invalid_event = EventLogEvent {
            event: "app-id".to_string(),
            event_payload: "".to_string(),
        };

        assert!(!verifier.validate_event(&invalid_event));
    }
}
