//! Emission schedule operations

use super::rows::ChallengeRow;
use super::PostgresStorageBackend;
use anyhow::Result;
use chrono::Utc;
use platform_api_models::*;
use std::collections::BTreeMap;
use tracing::info;
use uuid::Uuid;

impl PostgresStorageBackend {
    /// Get challenge emission schedule
    pub async fn get_challenge_emissions_impl(&self, id: Uuid) -> Result<EmissionSchedule> {
        let row = sqlx::query_as::<_, ChallengeRow>(
            "SELECT id, name, compose_hash, compose_yaml, version, images, resources, ports, env, emission_share, mechanism_id, weight, description, mermaid_chart, github_repo, created_at, updated_at FROM challenges WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Challenge not found"))?;

        Ok(EmissionSchedule {
            id: Id::from(row.id),
            emission_type: EmissionType::Challenge,
            challenge_id: Some(Id::from(row.id)),
            start_time: row.created_at,
            end_time: None,
            emission_rate: row.emission_share,
            total_amount: row.emission_share,
            distributed_amount: 0.0,
            status: EmissionStatus::Active,
            distribution_curve: DistributionCurve::Linear,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// List emission schedules with optional filters
    pub async fn list_emission_schedules_impl(
        &self,
        status: Option<String>,
        _emission_type: Option<String>,
        challenge_id: Option<Uuid>,
    ) -> Result<Vec<EmissionSchedule>> {
        let mut query = "SELECT id, name, compose_hash, compose_yaml, version, images, resources, ports, env, emission_share, mechanism_id, weight, description, mermaid_chart, github_repo, created_at, updated_at FROM challenges WHERE 1=1".to_string();
        let mut bind_counter = 1;

        if let Some(cid) = challenge_id {
            query.push_str(&format!(" AND id = ${}", bind_counter));
            bind_counter += 1;
        }

        let mut query_builder = sqlx::query_as::<_, ChallengeRow>(&query);
        if let Some(cid) = challenge_id {
            query_builder = query_builder.bind(cid);
        }

        let rows = query_builder.fetch_all(&self.pool).await?;

        let schedules: Vec<EmissionSchedule> = rows
            .into_iter()
            .map(|row| EmissionSchedule {
                id: Id::from(row.id),
                emission_type: EmissionType::Challenge,
                challenge_id: Some(Id::from(row.id)),
                start_time: row.created_at,
                end_time: None,
                emission_rate: row.emission_share,
                total_amount: row.emission_share,
                distributed_amount: 0.0,
                status: EmissionStatus::Active,
                distribution_curve: DistributionCurve::Linear,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();

        // Filter by status if provided
        let schedules = if let Some(status_str) = status {
            let status_enum = match status_str.as_str() {
                "scheduled" => EmissionStatus::Scheduled,
                "active" => EmissionStatus::Active,
                "completed" => EmissionStatus::Completed,
                "paused" => EmissionStatus::Paused,
                "cancelled" => EmissionStatus::Cancelled,
                _ => return Ok(schedules),
            };
            schedules
                .into_iter()
                .filter(|s| s.status == status_enum)
                .collect()
        } else {
            schedules
        };

        Ok(schedules)
    }

    /// Get emission schedule by ID
    pub async fn get_emission_schedule_impl(&self, id: Uuid) -> Result<EmissionSchedule> {
        self.get_challenge_emissions_impl(id).await
    }

    /// Create new emission schedule
    pub async fn create_emission_schedule_impl(
        &self,
        request: CreateEmissionScheduleRequest,
    ) -> Result<EmissionSchedule> {
        // For emissions, update the challenge's emission_share
        if let Some(challenge_id) = request.challenge_id {
            // Validate emission_rate is between 0 and 1
            if request.emission_rate < 0.0 || request.emission_rate > 1.0 {
                return Err(anyhow::anyhow!("Emission rate must be between 0.0 and 1.0"));
            }

            let now = Utc::now();
            sqlx::query("UPDATE challenges SET emission_share = $1, updated_at = $2 WHERE id = $3")
                .bind(request.emission_rate)
                .bind(now)
                .bind(challenge_id)
                .execute(&self.pool)
                .await?;

            self.get_challenge_emissions_impl(challenge_id).await
        } else {
            Err(anyhow::anyhow!(
                "challenge_id is required for emission schedules"
            ))
        }
    }

    /// Update emission schedule
    pub async fn update_emission_schedule_impl(
        &self,
        id: Uuid,
        request: UpdateEmissionScheduleRequest,
    ) -> Result<EmissionSchedule> {
        if let Some(emission_rate) = request.emission_rate {
            if !(0.0..=1.0).contains(&emission_rate) {
                return Err(anyhow::anyhow!("Emission rate must be between 0.0 and 1.0"));
            }

            sqlx::query("UPDATE challenges SET emission_share = $1, updated_at = $2 WHERE id = $3")
                .bind(emission_rate)
                .bind(Utc::now())
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        // Status is managed but not stored directly in challenges table
        // Challenges are always considered "Active" in this model

        self.get_challenge_emissions_impl(id).await
    }

    /// Distribute emission
    pub async fn distribute_emission_impl(
        &self,
        _id: Uuid,
        _request: DistributeEmissionRequest,
    ) -> Result<()> {
        // Distribution is handled by the weights mechanism, so we just log
        info!("Emission distribution requested for schedule {}, but distribution is handled automatically through weight aggregation", _id);
        Ok(())
    }

    /// Calculate emission for a specific challenge or all challenges
    pub async fn calculate_emission_impl(
        &self,
        request: CalculateEmissionRequest,
    ) -> Result<CalculateEmissionResponse> {
        let mut total_emission = 0.0;
        let mut breakdown = BTreeMap::new();

        if let Some(challenge_id) = request.challenge_id {
            let schedule = self.get_challenge_emissions_impl(challenge_id).await?;
            total_emission = schedule.emission_rate;
            breakdown.insert(
                format!("challenge_{}", challenge_id),
                schedule.emission_rate,
            );
        } else {
            // Calculate total emissions for all challenges if no specific challenge
            let schedules = self.list_emission_schedules_impl(None, None, None).await?;
            for schedule in schedules {
                total_emission += schedule.emission_rate;
                if let Some(cid) = schedule.challenge_id {
                    breakdown.insert(format!("challenge_{}", cid), schedule.emission_rate);
                }
            }
        }

        Ok(CalculateEmissionResponse {
            total_emission,
            breakdown,
            distributions: vec![],
            metrics: EmissionMetrics {
                participation_rate: 1.0,
                quality_score: 1.0,
                efficiency_score: 1.0,
                fairness_score: 1.0,
                sustainability_score: 1.0,
            },
        })
    }

    /// Get emission aggregate for a time period
    pub async fn get_emission_aggregate_impl(
        &self,
        period_start: chrono::DateTime<chrono::Utc>,
        period_end: chrono::DateTime<chrono::Utc>,
    ) -> Result<EmissionAggregate> {
        let schedules = self.list_emission_schedules_impl(None, None, None).await?;

        let mut total_emissions = 0.0;
        let mut challenge_emissions = 0.0;

        for schedule in &schedules {
            total_emissions += schedule.emission_rate;
            if schedule.emission_type == EmissionType::Challenge {
                challenge_emissions += schedule.emission_rate;
            }
        }

        Ok(EmissionAggregate {
            total_emissions,
            challenge_emissions,
            validator_emissions: 0.0,
            miner_emissions: 0.0,
            owner_emissions: 0.0,
            network_emissions: 0.0,
            period_start,
            period_end,
            distributions: vec![],
        })
    }

    /// Get challenge emission metrics
    pub async fn get_challenge_emission_metrics_impl(
        &self,
        id: Uuid,
    ) -> Result<ChallengeEmissionMetrics> {
        let schedule = self.get_challenge_emissions_impl(id).await?;

        Ok(ChallengeEmissionMetrics {
            challenge_id: Id::from(id),
            total_emission: schedule.total_amount,
            distributed_emission: schedule.distributed_amount,
            pending_emission: schedule.total_amount - schedule.distributed_amount,
            emission_rate: schedule.emission_rate,
            participation_score: 1.0,
            quality_score: 1.0,
            efficiency_score: 1.0,
            last_distribution: None,
            next_distribution: None,
        })
    }

    /// Get validator emission metrics
    pub async fn get_validator_emission_metrics_impl(
        &self,
        hotkey: &str,
    ) -> Result<ValidatorEmissionMetrics> {
        // Validators don't have direct emissions in this model
        Ok(ValidatorEmissionMetrics {
            validator_hotkey: hotkey.to_string(),
            total_emission: 0.0,
            distributed_emission: 0.0,
            pending_emission: 0.0,
            performance_score: 1.0,
            uptime_score: 1.0,
            accuracy_score: 1.0,
            efficiency_score: 1.0,
            last_distribution: None,
            next_distribution: None,
        })
    }

    /// Get miner emission metrics
    pub async fn get_miner_emission_metrics_impl(
        &self,
        hotkey: &str,
    ) -> Result<MinerEmissionMetrics> {
        // Miners don't have direct emissions in this model
        Ok(MinerEmissionMetrics {
            miner_hotkey: hotkey.to_string(),
            total_emission: 0.0,
            distributed_emission: 0.0,
            pending_emission: 0.0,
            submission_score: 1.0,
            quality_score: 1.0,
            participation_score: 1.0,
            innovation_score: 1.0,
            last_distribution: None,
            next_distribution: None,
        })
    }

    /// Get emission report for a time period
    pub async fn get_emission_report_impl(
        &self,
        period_start: chrono::DateTime<chrono::Utc>,
        period_end: chrono::DateTime<chrono::Utc>,
    ) -> Result<EmissionReport> {
        let aggregate = self
            .get_emission_aggregate_impl(period_start, period_end)
            .await?;
        let schedules = self.list_emission_schedules_impl(None, None, None).await?;

        Ok(EmissionReport {
            period_start,
            period_end,
            total_emissions: aggregate.total_emissions,
            schedule_count: schedules.len() as u32,
            distribution_count: 0,
            recipient_count: 0,
            avg_distribution_amount: 0.0,
            top_recipients: vec![],
            emission_trends: BTreeMap::new(),
        })
    }
}
