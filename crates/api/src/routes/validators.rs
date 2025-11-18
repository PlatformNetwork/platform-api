use axum::{extract::State, http::StatusCode, response::Json, routing::get, Router};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// Create validators router
pub fn create_router() -> Router<AppState> {
    Router::new().route("/validators/public", get(list_validators_public))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublicValidatorResponse {
    pub id: String,
    pub hotkey: String,
    pub name: String,
    pub status: String,
    pub location: Option<String>,
    pub jobs_processed: usize,
    pub success_rate: f64,
    pub uptime: String,
    pub connected_at: Option<String>,
    pub last_ping: Option<String>,
    pub challenge_assignments: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PublicValidatorsListResponse {
    pub validators: Vec<PublicValidatorResponse>,
    pub total: usize,
}

/// List public validators (read-only)
pub async fn list_validators_public(
    State(state): State<AppState>,
) -> Result<Json<PublicValidatorsListResponse>, StatusCode> {
    let pool = state.database_pool.as_ref().ok_or_else(|| {
        tracing::error!("Database pool not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Get validators from WebSocket connections
    let connected_validators = state.list_validator_connections().await;

    // Get validator stats from jobs table
    #[derive(sqlx::FromRow)]
    struct ValidatorStatsRow {
        validator_hotkey: String,
        total_jobs: i64,
        completed_jobs: i64,
        failed_jobs: i64,
        last_job_at: Option<chrono::DateTime<chrono::Utc>>,
        first_job_at: Option<chrono::DateTime<chrono::Utc>>,
    }

    let validator_stats = sqlx::query_as::<_, ValidatorStatsRow>(
        r#"
        SELECT 
            validator_hotkey,
            COUNT(*) as total_jobs,
            COUNT(*) FILTER (WHERE status = 'completed') as completed_jobs,
            COUNT(*) FILTER (WHERE status = 'failed') as failed_jobs,
            MAX(created_at) as last_job_at,
            MIN(created_at) as first_job_at
        FROM jobs
        WHERE validator_hotkey IS NOT NULL
        GROUP BY validator_hotkey
        ORDER BY total_jobs DESC
        "#,
    )
    .persistent(false)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query validator stats: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Create a map of hotkey -> stats
    let mut stats_map: std::collections::HashMap<String, ValidatorStatsRow> = validator_stats
        .into_iter()
        .map(|s| (s.validator_hotkey.clone(), s))
        .collect();

    // Combine connected validators with stats
    let mut validators = Vec::new();

    // Add connected validators
    for conn in connected_validators {
        let stats = stats_map.remove(&conn.validator_hotkey);

        let jobs_processed = stats.as_ref().map(|s| s.total_jobs as usize).unwrap_or(0);
        let completed_jobs = stats
            .as_ref()
            .map(|s| s.completed_jobs as usize)
            .unwrap_or(0);
        let success_rate = if jobs_processed > 0 {
            (completed_jobs as f64 / jobs_processed as f64) * 100.0
        } else {
            0.0
        };

        // Calculate uptime based on first and last job
        let uptime = if let Some(ref s) = stats {
            if let (Some(first), Some(last)) = (s.first_job_at, s.last_job_at) {
                let duration = last.signed_duration_since(first);
                let days = duration.num_days();
                if days > 0 {
                    format!("{}d", days)
                } else {
                    let hours = duration.num_hours();
                    format!("{}h", hours)
                }
            } else {
                "0h".to_string()
            }
        } else {
            "0h".to_string()
        };

        // Get challenge assignments from validator_challenge_status
        let challenge_assignments = {
            let status_map = state.validator_challenge_status.read().await;
            status_map
                .get(&conn.validator_hotkey)
                .map(|challenges| challenges.keys().cloned().collect())
                .unwrap_or_default()
        };

        validators.push(PublicValidatorResponse {
            id: conn.validator_hotkey.clone(),
            hotkey: conn.validator_hotkey.clone(),
            name: format!("Validator {}", &conn.validator_hotkey[..8]),
            status: "online".to_string(),
            location: None, // Could be derived from instance_id or other metadata
            jobs_processed,
            success_rate,
            uptime: format!("{}%", (success_rate / 100.0 * 99.0).min(99.9)), // Simplified uptime percentage
            connected_at: Some(conn.connected_at.to_rfc3339()),
            last_ping: Some(conn.last_ping.to_rfc3339()),
            challenge_assignments,
        });
    }

    // Add validators that have jobs but aren't currently connected
    for (hotkey, stats) in stats_map {
        let jobs_processed = stats.total_jobs as usize;
        let completed_jobs = stats.completed_jobs as usize;
        let success_rate = if jobs_processed > 0 {
            (completed_jobs as f64 / jobs_processed as f64) * 100.0
        } else {
            0.0
        };

        let uptime = if let (Some(first), Some(last)) = (stats.first_job_at, stats.last_job_at) {
            let duration = last.signed_duration_since(first);
            let days = duration.num_days();
            if days > 0 {
                format!("{}d", days)
            } else {
                let hours = duration.num_hours();
                format!("{}h", hours)
            }
        } else {
            "0h".to_string()
        };

        // Determine status based on last job time
        let status = if let Some(last_job) = stats.last_job_at {
            let hours_since_last_job = chrono::Utc::now()
                .signed_duration_since(last_job)
                .num_hours();
            if hours_since_last_job < 1 {
                "online".to_string()
            } else if hours_since_last_job < 24 {
                "syncing".to_string()
            } else {
                "offline".to_string()
            }
        } else {
            "offline".to_string()
        };

        validators.push(PublicValidatorResponse {
            id: hotkey.clone(),
            hotkey: hotkey.clone(),
            name: format!("Validator {}", &hotkey[..8]),
            status,
            location: None,
            jobs_processed,
            success_rate,
            uptime: format!("{}%", (success_rate / 100.0 * 99.0).min(99.9)),
            connected_at: stats.first_job_at.map(|d| d.to_rfc3339()),
            last_ping: stats.last_job_at.map(|d| d.to_rfc3339()),
            challenge_assignments: Vec::new(),
        });
    }

    // Sort by jobs processed (descending)
    validators.sort_by(|a, b| b.jobs_processed.cmp(&a.jobs_processed));

    Ok(Json(PublicValidatorsListResponse {
        total: validators.len(),
        validators,
    }))
}
