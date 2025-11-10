use super::{Hotkey, Id, Score};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Emission type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EmissionType {
    Challenge,
    Validator,
    Miner,
    Owner,
    Network,
}

/// Emission status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EmissionStatus {
    Scheduled,
    Active,
    Completed,
    Paused,
    Cancelled,
}

/// Emission schedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionSchedule {
    pub id: Id,
    pub emission_type: EmissionType,
    pub challenge_id: Option<Id>,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub emission_rate: f64,
    pub total_amount: f64,
    pub distributed_amount: f64,
    pub status: EmissionStatus,
    pub distribution_curve: DistributionCurve,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Default for EmissionSchedule {
    fn default() -> Self {
        Self {
            id: Id::new_v4(),
            emission_type: EmissionType::Challenge,
            challenge_id: None,
            start_time: Utc::now(),
            end_time: None,
            emission_rate: 1.0,
            total_amount: 1000.0,
            distributed_amount: 0.0,
            status: EmissionStatus::Scheduled,
            distribution_curve: DistributionCurve::Linear,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

/// Distribution curve for emissions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DistributionCurve {
    Linear,
    Exponential {
        decay_factor: f64,
    },
    Step {
        intervals: Vec<(DateTime<Utc>, f64)>,
    },
    Custom {
        points: Vec<(f64, f64)>,
    },
}

/// Emission distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionDistribution {
    pub schedule_id: Id,
    pub recipient_hotkey: Hotkey,
    pub amount: f64,
    pub percentage: f64,
    pub distributed_at: DateTime<Utc>,
    pub transaction_hash: Option<String>,
    pub receipt: String,
}

/// Emission aggregate
#[derive(Debug, Serialize, Deserialize)]
pub struct EmissionAggregate {
    pub total_emissions: f64,
    pub challenge_emissions: f64,
    pub validator_emissions: f64,
    pub miner_emissions: f64,
    pub owner_emissions: f64,
    pub network_emissions: f64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub distributions: Vec<EmissionDistribution>,
}

/// Challenge emission metrics
#[derive(Debug, Serialize, Deserialize)]
pub struct ChallengeEmissionMetrics {
    pub challenge_id: Id,
    pub total_emission: f64,
    pub distributed_emission: f64,
    pub pending_emission: f64,
    pub emission_rate: f64,
    pub participation_score: f64,
    pub quality_score: f64,
    pub efficiency_score: f64,
    pub last_distribution: Option<DateTime<Utc>>,
    pub next_distribution: Option<DateTime<Utc>>,
}

/// Validator emission metrics
#[derive(Debug, Serialize, Deserialize)]
pub struct ValidatorEmissionMetrics {
    pub validator_hotkey: Hotkey,
    pub total_emission: f64,
    pub distributed_emission: f64,
    pub pending_emission: f64,
    pub performance_score: f64,
    pub uptime_score: f64,
    pub accuracy_score: f64,
    pub efficiency_score: f64,
    pub last_distribution: Option<DateTime<Utc>>,
    pub next_distribution: Option<DateTime<Utc>>,
}

/// Miner emission metrics
#[derive(Debug, Serialize, Deserialize)]
pub struct MinerEmissionMetrics {
    pub miner_hotkey: Hotkey,
    pub total_emission: f64,
    pub distributed_emission: f64,
    pub pending_emission: f64,
    pub submission_score: f64,
    pub quality_score: f64,
    pub participation_score: f64,
    pub innovation_score: f64,
    pub last_distribution: Option<DateTime<Utc>>,
    pub next_distribution: Option<DateTime<Utc>>,
}

/// Emission calculation request
#[derive(Debug, Serialize, Deserialize)]
pub struct CalculateEmissionRequest {
    pub challenge_id: Option<Id>,
    pub validator_hotkey: Option<Hotkey>,
    pub miner_hotkey: Option<Hotkey>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub include_pending: bool,
}

/// Emission calculation response
#[derive(Debug, Serialize, Deserialize)]
pub struct CalculateEmissionResponse {
    pub total_emission: f64,
    pub breakdown: BTreeMap<String, f64>,
    pub distributions: Vec<EmissionDistribution>,
    pub metrics: EmissionMetrics,
}

/// Emission metrics
#[derive(Debug, Serialize, Deserialize)]
pub struct EmissionMetrics {
    pub participation_rate: f64,
    pub quality_score: f64,
    pub efficiency_score: f64,
    pub fairness_score: f64,
    pub sustainability_score: f64,
}

/// Emission schedule request
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEmissionScheduleRequest {
    pub emission_type: EmissionType,
    pub challenge_id: Option<Id>,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub emission_rate: f64,
    pub total_amount: f64,
    pub distribution_curve: DistributionCurve,
}

/// Emission schedule update request
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateEmissionScheduleRequest {
    pub schedule_id: Id,
    pub emission_rate: Option<f64>,
    pub total_amount: Option<f64>,
    pub end_time: Option<DateTime<Utc>>,
    pub status: Option<EmissionStatus>,
    pub distribution_curve: Option<DistributionCurve>,
}

/// Emission distribution request
#[derive(Debug, Serialize, Deserialize)]
pub struct DistributeEmissionRequest {
    pub schedule_id: Id,
    pub recipients: Vec<EmissionRecipient>,
    pub distribution_time: Option<DateTime<Utc>>,
    pub batch_size: Option<u32>,
}

/// Emission recipient
#[derive(Debug, Serialize, Deserialize)]
pub struct EmissionRecipient {
    pub hotkey: Hotkey,
    pub amount: f64,
    pub percentage: f64,
    pub reason: String,
}

/// Emission audit log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionAuditLog {
    pub id: Id,
    pub schedule_id: Id,
    pub event_type: EmissionEventType,
    pub amount: f64,
    pub recipient_hotkey: Option<Hotkey>,
    pub timestamp: DateTime<Utc>,
    pub details: BTreeMap<String, String>,
    pub receipt: String,
}

/// Emission event type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EmissionEventType {
    ScheduleCreated,
    ScheduleUpdated,
    DistributionStarted,
    DistributionCompleted,
    DistributionFailed,
    SchedulePaused,
    ScheduleResumed,
    ScheduleCancelled,
}

/// Emission report
#[derive(Debug, Serialize, Deserialize)]
pub struct EmissionReport {
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub total_emissions: f64,
    pub schedule_count: u32,
    pub distribution_count: u32,
    pub recipient_count: u32,
    pub avg_distribution_amount: f64,
    pub top_recipients: Vec<EmissionRecipient>,
    pub emission_trends: BTreeMap<String, f64>,
}
