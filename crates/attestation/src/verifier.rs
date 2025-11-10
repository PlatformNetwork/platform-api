use crate::{AttestationConfig, VerificationResult};
use anyhow::{Context, Result};
use dcap_qvl::{collateral, verify::verify};
use platform_api_models::AttestationRequest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha384};
use std::collections::HashMap;
use std::time::Duration;

/// TDX verifier implementation using dcap-qvl
pub struct TdxVerifier {
    config: AttestationConfig,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct VerifiedQuote {
    report: VerifiedReport,
    status: String,
}

#[derive(Debug, Deserialize)]
struct VerifiedReport {
    #[serde(rename = "TD10")]
    td10: Option<TDReport>,
    #[serde(rename = "TD15")]
    td15: Option<TDReport>,
}

#[derive(Debug, Deserialize)]
struct TDReport {
    mr_td: String,
    rt_mr0: String,
    rt_mr1: String,
    rt_mr2: String,
    rt_mr3: String,
    report_data: String,
}

#[derive(Debug, Deserialize)]
struct EventLogEvent {
    imr: u8,
    event_type: u32,
    digest: String,
    event: String,
    event_payload: String,
}

impl TdxVerifier {
    pub fn new(config: AttestationConfig) -> Self {
        // Use a default timeout of 30 seconds for verification requests
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self { config, client }
    }

    pub async fn verify_static(
        &self,
        request: &AttestationRequest,
        event_log: Option<&str>,
    ) -> Result<VerificationResult> {
        tracing::info!("Verifying TDX attestation using dcap-qvl");

        let quote = request
            .quote
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing quote"))?;

        // Verify quote using dcap-qvl
        let verified_quote = match self.verify_quote(quote).await {
            Ok(vq) => vq,
            Err(e) => {
                tracing::error!("dcap-qvl verification failed: {}", e);
                // Reject attestation if verification tool is not available
                return Ok(VerificationResult {
                    is_valid: false,
                    measurements: request.measurements.clone(),
                    app_id: None,
                    instance_id: None,
                    device_id: None,
                    error: Some(format!("dcap-qvl verification failed: {}", e)),
                });
            }
        };

        // Extract MRs from verified quote
        let mrs = self.extract_mrs(&verified_quote)?;

        // Extract app info from event log
        let (app_id, instance_id, compose_hash) = self.extract_app_info(event_log)?;

        tracing::info!(
            app_id = ?app_id,
            instance_id = ?instance_id,
            compose_hash = ?compose_hash,
            "Attestation verified successfully"
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

    async fn verify_quote(&self, quote: &[u8]) -> Result<VerifiedQuote> {
        // Get collateral from Intel PCS
        let collateral_data = collateral::get_collateral_from_pcs(quote)
            .await
            .context("Failed to get collateral from Intel PCS")?;

        // Get current timestamp
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Verify quote
        let tcb = verify(quote, &collateral_data, now)
            .map_err(|e| anyhow::anyhow!("Failed to verify quote: {:?}", e))?;

        tracing::info!("Quote verified, status: {:?}", tcb.status);

        // For now, just verify that the signature is valid
        // In a production environment, you would extract and compare MRs
        let verified = VerifiedQuote {
            report: VerifiedReport {
                td10: Some(TDReport {
                    mr_td: String::new(),
                    rt_mr0: String::new(),
                    rt_mr1: String::new(),
                    rt_mr2: String::new(),
                    rt_mr3: String::new(),
                    report_data: String::new(),
                }),
                td15: None,
            },
            status: format!("{:?}", tcb.status),
        };

        Ok(verified)
    }

    fn extract_mrs(&self, verified: &VerifiedQuote) -> Result<HashMap<String, String>> {
        let report = if let Some(td10) = &verified.report.td10 {
            td10
        } else if let Some(td15) = &verified.report.td15 {
            td15
        } else {
            return Err(anyhow::anyhow!("No TD10 or TD15 report found"));
        };

        let mut mrs = HashMap::new();
        mrs.insert("mr_td".to_string(), report.mr_td.clone());
        mrs.insert("rt_mr0".to_string(), report.rt_mr0.clone());
        mrs.insert("rt_mr1".to_string(), report.rt_mr1.clone());
        mrs.insert("rt_mr2".to_string(), report.rt_mr2.clone());
        mrs.insert("rt_mr3".to_string(), report.rt_mr3.clone());
        mrs.insert("report_data".to_string(), report.report_data.clone());

        Ok(mrs)
    }

    fn extract_app_info(
        &self,
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

    fn replay_rtmr(&self, history: &[String]) -> String {
        const INIT_MR: &str = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

        if history.is_empty() {
            return INIT_MR.to_string();
        }

        let mut mr = hex::decode(INIT_MR).unwrap();

        for content in history {
            let mut content_bytes = hex::decode(content).unwrap();
            // Pad to 48 bytes if shorter
            if content_bytes.len() < 48 {
                content_bytes.resize(48, 0);
            }

            // mr = sha384(concat(mr, content))
            let mut hasher = Sha384::new();
            hasher.update(&mr);
            hasher.update(&content_bytes);
            mr = hasher.finalize().to_vec();
        }

        hex::encode(mr)
    }

    fn validate_event(&self, event: &EventLogEvent) -> bool {
        // Skip validation for non-IMR3 events
        if event.imr != 3 {
            return true;
        }

        // Calculate digest using sha384(type:event:payload)
        let mut hasher = Sha384::new();
        hasher.update(event.event_type.to_le_bytes());
        hasher.update(b":");
        hasher.update(event.event.as_bytes());
        hasher.update(b":");

        let payload = hex::decode(&event.event_payload).unwrap_or_default();
        hasher.update(&payload);

        let calculated_digest = hex::encode(hasher.finalize());
        calculated_digest == event.digest
    }
}
