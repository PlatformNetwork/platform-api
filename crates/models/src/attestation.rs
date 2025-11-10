use super::{Id, KeyMaterial, Measurement, Nonce, Policy, Quote, Receipt, Report, SessionToken};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Attestation type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AttestationType {
    SgxDcap,
    SevSnp,
    Tdx,
}

/// Attestation status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AttestationStatus {
    Pending,
    Verified,
    Failed,
    Expired,
}

/// Attestation request
#[derive(Debug, Serialize, Deserialize)]
pub struct AttestationRequest {
    pub attestation_type: AttestationType,
    pub quote: Option<Quote>,
    pub report: Option<Report>,
    pub nonce: Nonce,
    pub measurements: Vec<Measurement>,
    pub capabilities: Vec<String>,
}

/// Attestation response
#[derive(Debug, Serialize, Deserialize)]
pub struct AttestationResponse {
    pub session_token: SessionToken,
    pub status: AttestationStatus,
    pub expires_at: DateTime<Utc>,
    pub verified_measurements: Vec<Measurement>,
    pub policy: Policy,
    pub error: Option<String>,
}

/// Key release request
#[derive(Debug, Serialize, Deserialize)]
pub struct KeyReleaseRequest {
    pub session_token: SessionToken,
    pub policy: Policy,
    pub harness_digest: String,
    pub measurements: Vec<Measurement>,
    pub nonce: Nonce,
}

/// Key release response
#[derive(Debug, Serialize, Deserialize)]
pub struct KeyReleaseResponse {
    pub sealed_key: KeyMaterial,
    pub key_id: String,
    pub expires_at: DateTime<Utc>,
    pub policy: Policy,
    pub error: Option<String>,
}

/// Attestation session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationSession {
    pub id: Id,
    pub session_token: SessionToken,
    pub attestation_type: AttestationType,
    pub status: AttestationStatus,
    pub validator_hotkey: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub verified_measurements: Vec<Measurement>,
    pub policy: Policy,
    pub key_releases: Vec<KeyRelease>,
}

/// Key release record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRelease {
    pub id: Id,
    pub session_id: Id,
    pub key_id: String,
    pub harness_digest: String,
    pub released_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub policy: Policy,
    pub receipt: Receipt,
}

/// Attestation policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationPolicy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub attestation_type: AttestationType,
    pub allowed_measurements: Vec<Measurement>,
    pub allowed_digests: Vec<String>,
    pub tcb_requirements: TcbRequirements,
    pub nonce_freshness: u64,
    pub key_derivation: KeyDerivationPolicy,
}

/// TCB (Trusted Computing Base) requirements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcbRequirements {
    pub min_svn: Option<u32>,
    pub max_svn: Option<u32>,
    pub allowed_svns: Vec<u32>,
    pub min_tcb_version: Option<String>,
    pub max_tcb_version: Option<String>,
}

/// Key derivation policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyDerivationPolicy {
    pub algorithm: String,
    pub key_size: u32,
    pub derivation_context: String,
    pub usage_count: Option<u32>,
    pub time_bound: Option<u64>,
}

/// Attestation verification result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationVerificationResult {
    pub is_valid: bool,
    pub measurements_match: bool,
    pub tcb_valid: bool,
    pub nonce_fresh: bool,
    pub error: Option<String>,
    pub details: AttestationDetails,
}

/// Attestation verification details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationDetails {
    pub quote_valid: bool,
    pub signature_valid: bool,
    pub certificate_chain_valid: bool,
    pub measurements: Vec<Measurement>,
    pub tcb_info: Option<TcbInfo>,
    pub timestamp: DateTime<Utc>,
}

/// TCB information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcbInfo {
    pub version: String,
    pub svn: u32,
    pub components: Vec<TcbComponent>,
    pub status: String,
}

/// TCB component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcbComponent {
    pub component_id: u8,
    pub svn: u32,
    pub category: String,
}

/// Attestation audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationAuditLog {
    pub id: Id,
    pub session_id: Option<Id>,
    pub event_type: AttestationEventType,
    pub validator_hotkey: String,
    pub timestamp: DateTime<Utc>,
    pub details: std::collections::BTreeMap<String, String>,
    pub receipt: Receipt,
}

/// Attestation event type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AttestationEventType {
    AttestationRequested,
    AttestationVerified,
    AttestationFailed,
    KeyReleased,
    KeyExpired,
    PolicyViolation,
    SessionExpired,
}
