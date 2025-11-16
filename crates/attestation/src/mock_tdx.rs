use anyhow::{Context, Result};
use rand::RngCore;
use sha2::{Digest, Sha256, Sha384};

/// Mock TDX quote generator for development mode
/// 
/// Generates structurally valid TDX quotes that simulate real TDX behavior
/// including proper report_data placement, measurements, and event logs.
pub struct MockTdxQuote;

/// TDX quote structure for mock generation
#[derive(Debug, Clone)]
pub struct MockQuoteData {
    /// The generated quote bytes
    pub quote: Vec<u8>,
    /// Event log JSON string
    pub event_log: String,
    /// RTMR values (RTMR0-3)
    pub rtmrs: Vec<String>,
}

/// Measurement data extracted from quote
#[derive(Debug, Clone)]
pub struct MeasurementData {
    pub mr_td: String,
    pub rt_mr0: String,
    pub rt_mr1: String,
    pub rt_mr2: String,
    pub rt_mr3: String,
    pub report_data: Vec<u8>,
}

impl MockTdxQuote {
    /// Generate a mock TDX quote with proper structure
    /// 
    /// # Arguments
    /// * `nonce` - Nonce bytes to bind in report_data (SHA256 hash will be embedded)
    /// * `compose_hash` - Docker compose hash for event log
    /// * `app_id` - Application identifier
    /// * `instance_id` - Instance identifier
    /// 
    /// # Returns
    /// A MockQuoteData containing the quote, event log, and RTMRs
    pub fn generate(
        nonce: &[u8],
        compose_hash: Option<&str>,
        app_id: Option<&str>,
        instance_id: Option<&str>,
    ) -> Result<MockQuoteData> {
        // Calculate report_data from nonce (SHA256)
        let mut hasher = Sha256::new();
        hasher.update(nonce);
        let report_data = hasher.finalize().to_vec();

        // Generate a mock quote with proper size (TDX quotes are typically 1024+ bytes)
        // We'll use 1024 bytes as minimum, matching real TDX quote structure
        let mut quote = vec![0u8; 1024];
        let mut rng = rand::thread_rng();
        rng.fill_bytes(&mut quote);

        // Embed report_data at known offsets (common TDX quote offsets)
        // Try multiple common offsets to match real TDX behavior
        let report_offsets: [usize; 3] = [368, 568, 576];
        
        // Use the first offset that fits
        let report_offset = report_offsets[0];
        if quote.len() >= report_offset + 32 {
            quote[report_offset..report_offset + 32].copy_from_slice(&report_data);
        }

        // Generate realistic measurements (MRs)
        let _mr_td = Self::generate_mr_hash("td");
        let rt_mr0 = Self::generate_rtmr(&[]);
        let rt_mr1 = Self::generate_rtmr(&[b"kernel"]);
        let rt_mr2 = Self::generate_rtmr(&[b"initrd"]);
        
        // RTMR3 contains event log measurements
        let event_log_events = Self::build_event_log_events(compose_hash, app_id, instance_id);
        let rt_mr3 = Self::generate_rtmr_from_events(&event_log_events);

        // Build event log JSON
        let event_log = Self::build_event_log_json(&event_log_events);

        // Build RTMRs list
        let rtmrs = vec![
            rt_mr0.clone(),
            rt_mr1.clone(),
            rt_mr2.clone(),
            rt_mr3.clone(),
        ];

        Ok(MockQuoteData {
            quote,
            event_log,
            rtmrs,
        })
    }

    /// Generate a mock quote with default values
    pub fn generate_default(nonce: &[u8]) -> Result<MockQuoteData> {
        Self::generate(nonce, None, None, None)
    }

    /// Extract measurement data from a mock quote
    /// 
    /// This simulates the extraction process that would happen with a real TDX quote
    pub fn extract_measurements(quote: &[u8], nonce: &[u8]) -> Result<MeasurementData> {
        // Calculate expected report_data
        let mut hasher = Sha256::new();
        hasher.update(nonce);
        let expected_report_data = hasher.finalize().to_vec();

        // Try to extract report_data from common offsets
        let report_offsets: [usize; 3] = [368, 568, 576];
        let mut found_report_data = None;

        for offset in &report_offsets {
            if quote.len() >= *offset + 32 {
                let candidate = &quote[*offset..*offset + 32];
                if candidate == expected_report_data.as_slice() {
                    found_report_data = Some(candidate.to_vec());
                    break;
                }
            }
        }

        let report_data = found_report_data
            .context("Could not find report_data matching nonce")?;

        // Generate mock MRs (in real TDX, these would be extracted from quote)
        let mr_td = Self::generate_mr_hash("td");
        let rt_mr0 = Self::generate_rtmr(&[]);
        let rt_mr1 = Self::generate_rtmr(&[b"kernel"]);
        let rt_mr2 = Self::generate_rtmr(&[b"initrd"]);
        let rt_mr3 = Self::generate_rtmr(&[b"event_log"]);

        Ok(MeasurementData {
            mr_td,
            rt_mr0,
            rt_mr1,
            rt_mr2,
            rt_mr3,
            report_data,
        })
    }

    /// Build event log events from provided data
    fn build_event_log_events(
        compose_hash: Option<&str>,
        app_id: Option<&str>,
        instance_id: Option<&str>,
    ) -> Vec<EventLogEvent> {
        let mut events = Vec::new();

        // Add app_id event if provided
        if let Some(app_id) = app_id {
            events.push(EventLogEvent {
                imr: 3,
                event_type: 1,
                event: "app-id".to_string(),
                event_payload: app_id.to_string(),
            });
        }

        // Add instance_id event if provided
        if let Some(instance_id) = instance_id {
            events.push(EventLogEvent {
                imr: 3,
                event_type: 2,
                event: "instance-id".to_string(),
                event_payload: instance_id.to_string(),
            });
        }

        // Add compose_hash event if provided
        if let Some(compose_hash) = compose_hash {
            events.push(EventLogEvent {
                imr: 3,
                event_type: 3,
                event: "compose-hash".to_string(),
                event_payload: compose_hash.to_string(),
            });
        }

        // Always add dev_mode marker
        events.push(EventLogEvent {
            imr: 3,
            event_type: 4,
            event: "dev-mode".to_string(),
            event_payload: "true".to_string(),
        });

        events
    }

    /// Build event log JSON from events
    fn build_event_log_json(events: &[EventLogEvent]) -> String {
        let event_log_array: Vec<serde_json::Value> = events
            .iter()
            .map(|e| {
                // Calculate digest for event
                let mut hasher = Sha384::new();
                hasher.update(e.event_type.to_le_bytes());
                hasher.update(b":");
                hasher.update(e.event.as_bytes());
                hasher.update(b":");
                hasher.update(e.event_payload.as_bytes());
                let digest = hex::encode(hasher.finalize());

                serde_json::json!({
                    "imr": e.imr,
                    "event_type": e.event_type,
                    "digest": digest,
                    "event": e.event,
                    "event_payload": e.event_payload,
                })
            })
            .collect();

        serde_json::to_string(&event_log_array)
            .unwrap_or_else(|_| "[]".to_string())
    }

    /// Generate RTMR from content
    fn generate_rtmr(content: &[&[u8]]) -> String {
        const INIT_MR: &str = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

        if content.is_empty() {
            return INIT_MR.to_string();
        }

        let mut mr = hex::decode(INIT_MR).unwrap_or_default();
        if mr.is_empty() {
            mr = vec![0u8; 48]; // SHA384 produces 48 bytes
        }

        for item in content {
            let mut item_bytes = item.to_vec();
            // Pad to 48 bytes if shorter
            if item_bytes.len() < 48 {
                item_bytes.resize(48, 0);
            }

            // mr = sha384(concat(mr, content))
            let mut hasher = Sha384::new();
            hasher.update(&mr);
            hasher.update(&item_bytes);
            mr = hasher.finalize().to_vec();
        }

        hex::encode(mr)
    }

    /// Generate RTMR from event log events
    fn generate_rtmr_from_events(events: &[EventLogEvent]) -> String {
        const INIT_MR: &str = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

        let mut mr = hex::decode(INIT_MR).unwrap_or_default();
        if mr.is_empty() {
            mr = vec![0u8; 48];
        }

        for event in events {
            // Calculate digest for event
            let mut hasher = Sha384::new();
            hasher.update(event.event_type.to_le_bytes());
            hasher.update(b":");
            hasher.update(event.event.as_bytes());
            hasher.update(b":");
            hasher.update(event.event_payload.as_bytes());
            let digest = hasher.finalize().to_vec();

            // Update MR with event digest
            let mut hasher = Sha384::new();
            hasher.update(&mr);
            hasher.update(&digest);
            mr = hasher.finalize().to_vec();
        }

        hex::encode(mr)
    }

    /// Generate a mock MR hash
    fn generate_mr_hash(seed: &str) -> String {
        let mut hasher = Sha384::new();
        hasher.update(seed.as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// Event log event structure
#[derive(Debug, Clone)]
struct EventLogEvent {
    imr: u8,
    event_type: u32,
    event: String,
    event_payload: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_mock_quote() {
        let nonce = b"test-nonce-12345";
        let result = MockTdxQuote::generate_default(nonce).unwrap();
        
        assert_eq!(result.quote.len(), 1024);
        assert!(!result.event_log.is_empty());
        assert_eq!(result.rtmrs.len(), 4);
    }

    #[test]
    fn test_extract_measurements() {
        let nonce = b"test-nonce-12345";
        let quote_data = MockTdxQuote::generate_default(nonce).unwrap();
        
        let measurements = MockTdxQuote::extract_measurements(&quote_data.quote, nonce).unwrap();
        
        assert_eq!(measurements.report_data.len(), 32);
        assert!(!measurements.mr_td.is_empty());
        assert_eq!(measurements.rt_mr0.len(), 96); // 48 bytes * 2 (hex)
    }

    #[test]
    fn test_nonce_binding() {
        let nonce1 = b"nonce-1";
        let nonce2 = b"nonce-2";
        
        let quote1 = MockTdxQuote::generate_default(nonce1).unwrap();
        let quote2 = MockTdxQuote::generate_default(nonce2).unwrap();
        
        // Quotes should be different
        assert_ne!(quote1.quote, quote2.quote);
        
        // But report_data should match respective nonces
        let m1 = MockTdxQuote::extract_measurements(&quote1.quote, nonce1).unwrap();
        let m2 = MockTdxQuote::extract_measurements(&quote2.quote, nonce2).unwrap();
        
        assert_ne!(m1.report_data, m2.report_data);
    }

    #[test]
    fn test_event_log_generation() {
        let nonce = b"test-nonce";
        let quote_data = MockTdxQuote::generate(
            nonce,
            Some("test-compose-hash"),
            Some("test-app-id"),
            Some("test-instance-id"),
        ).unwrap();
        
        let event_log: serde_json::Value = serde_json::from_str(&quote_data.event_log).unwrap();
        assert!(event_log.is_array());
        
        let events = event_log.as_array().unwrap();
        assert!(events.len() >= 3); // app-id, instance-id, compose-hash, dev-mode
    }
}

