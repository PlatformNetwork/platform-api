use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Query parameters for listing challenges
#[derive(Debug, Deserialize)]
pub struct ListChallengesParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub status: Option<String>,
    pub visibility: Option<String>,
}

/// Query parameters for challenge jobs
#[derive(Debug, Deserialize)]
pub struct ChallengeJobsParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub status: Option<String>,
    pub include_test_results: Option<bool>,
}

/// Request body for storing challenge environment variables
#[derive(Debug, Deserialize)]
pub struct StoreChallengeEnvVarsRequest {
    pub env_vars: std::collections::HashMap<String, String>,
}

/// Public challenge response structure
#[derive(Debug, Serialize, Deserialize)]
pub struct PublicChallengeResponse {
    pub id: String,
    pub name: String,
    pub status: String,
    pub description: Option<String>,
    pub difficulty: Option<String>,
    pub mechanism_id: i16,
    pub emission_share: f64,
    pub github_repo: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub stats: ChallengeStats,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChallengeStats {
    pub participant_count: usize,
    pub jobs_processed: usize,
    pub success_rate: f64,
    pub pool_size_tao: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublicChallengeListResponse {
    pub challenges: Vec<PublicChallengeResponse>,
    pub total: usize,
}

/// Database row for challenges
#[derive(sqlx::FromRow)]
pub(crate) struct ChallengeRow {
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
    pub mechanism_id: i16,
    pub weight: Option<f64>,
    pub description: Option<String>,
    pub mermaid_chart: Option<String>,
    pub github_repo: Option<String>,
    pub dstack_image: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Minimal database row for active challenges
#[derive(sqlx::FromRow)]
pub(crate) struct MinimalChallengeRow {
    pub id: Uuid,
    pub name: String,
    pub compose_hash: String,
    pub github_repo: Option<String>,
    pub mechanism_id: i16,
    pub emission_share: f64,
    pub resources: serde_json::Value,
}

/// Minimal database row for public challenges
#[derive(sqlx::FromRow)]
pub(crate) struct PublicChallengeRow {
    pub id: Uuid,
    pub name: String,
    pub mechanism_id: i16,
    pub emission_share: f64,
    pub description: Option<String>,
    pub github_repo: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

