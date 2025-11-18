//! Database row structures for PostgreSQL

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Database row for pools table
#[derive(Debug, FromRow)]
pub struct PoolRow {
    pub id: Uuid,
    pub validator_hotkey: String,
    pub name: String,
    pub description: Option<String>,
    pub autoscale_policy: serde_json::Value,
    pub region: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Database row for nodes table  
#[derive(Debug, FromRow)]
pub struct NodeRow {
    pub id: Uuid,
    pub pool_id: Uuid,
    pub name: String,
    pub vmm_url: String,
    pub capacity: serde_json::Value,
    pub health: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Database row for node capacity queries
#[derive(Debug, FromRow)]
pub struct NodeCapacityRow {
    pub capacity: serde_json::Value,
    pub health: serde_json::Value,
}

/// Database row for challenges table
#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct ChallengeRow {
    pub id: Uuid,
    pub name: String,
    pub compose_hash: String,
    pub compose_yaml: String,
    pub version: String,
    pub images: Vec<String>,
    pub resources: serde_json::Value,
    pub ports: serde_json::Value,
    pub env: serde_json::Value,
    pub emission_share: f64,
    pub mechanism_id: i16, // PostgreSQL SMALLINT maps to i16 in Rust, but we'll convert to u8
    pub weight: Option<f64>,
    pub description: Option<String>,
    pub mermaid_chart: Option<String>,
    pub github_repo: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Database row for VM compose configs
#[derive(Debug, FromRow)]
pub struct VmComposeRow {
    pub id: Uuid,
    pub vm_type: String,
    pub compose_content: String,
    pub description: Option<String>,
    pub required_env: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
