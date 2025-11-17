use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

pub mod attestation;
pub mod challenge;
pub mod config;
pub mod emissions;
pub mod errors;
pub mod job;
pub mod pool;
pub mod vm_compose;

pub use attestation::*;
pub use challenge::*;
pub use config::*;
pub use emissions::*;
pub use errors::*;
pub use job::*;
pub use pool::*;
pub use vm_compose::*;

// Type aliases for backwards compatibility
pub type TSubnetConfig = SubnetConfig;
pub type EmissionSchedule = emissions::EmissionSchedule;
pub type EmissionsSchedule = emissions::EmissionSchedule;

/// Common identifier type
pub type Id = Uuid;

/// Hotkey type for Bittensor integration
pub type Hotkey = String;

/// Score type for evaluation results
pub type Score = f64;

/// Digest type for content verification
pub type Digest = String;

/// Nonce type for attestation
pub type Nonce = Vec<u8>;

/// Quote type for SGX attestation
pub type Quote = Vec<u8>;

/// Report type for SEV-SNP attestation
pub type Report = Vec<u8>;

/// Measurement type for TEE verification
pub type Measurement = Vec<u8>;

/// Policy type for key release
pub type Policy = String;

/// Session token for authenticated operations
pub type SessionToken = String;

/// Key material for encryption/decryption
pub type KeyMaterial = Vec<u8>;

/// Receipt type for audit trails
pub type Receipt = String;
