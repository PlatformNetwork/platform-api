use axum::{extract::State, http::StatusCode, response::Json, routing::get, Router};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// Create network router
pub fn create_router() -> Router<AppState> {
    Router::new().route("/network/overview", get(get_network_overview))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkOverviewResponse {
    pub total_challenges: usize,
    pub active_challenges: usize,
    pub total_validators: usize,
    pub active_validators: usize,
    pub total_jobs_processed: usize,
    pub total_emissions_percentage: f64,
    pub mechanisms_summary: Vec<MechanismSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MechanismSummary {
    pub id: String,
    pub name: String,
    pub emission_percentage: f64,
    pub challenge_count: usize,
    pub status: String,
}

/// Get network-wide statistics
pub async fn get_network_overview(
    State(state): State<AppState>,
) -> Result<Json<NetworkOverviewResponse>, StatusCode> {
    let pool = state.database_pool.as_ref().ok_or_else(|| {
        tracing::error!("Database pool not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Get total challenges count
    let total_challenges: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM challenges")
        .persistent(false)
        .fetch_one(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to count challenges: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Get active challenges count (challenges that have jobs)
    let active_challenges: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(DISTINCT challenge_id) 
        FROM jobs 
        WHERE created_at > NOW() - INTERVAL '7 days'
        "#,
    )
    .persistent(false)
    .fetch_one(pool.as_ref())
    .await
    .unwrap_or(0);

    // Get total jobs processed
    let total_jobs_processed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .persistent(false)
        .fetch_one(pool.as_ref())
        .await
        .unwrap_or(0);

    // Get mechanisms summary
    #[derive(sqlx::FromRow)]
    struct MechanismRow {
        mechanism_id: i16,
        challenge_count: i64,
        total_emission_share: f64,
    }

    let mechanisms_data = sqlx::query_as::<_, MechanismRow>(
        r#"
        SELECT 
            mechanism_id,
            COUNT(*) as challenge_count,
            SUM(emission_share) as total_emission_share
        FROM challenges
        GROUP BY mechanism_id
        ORDER BY mechanism_id
        "#,
    )
    .persistent(false)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query mechanisms: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mechanisms_summary: Vec<MechanismSummary> = mechanisms_data
        .into_iter()
        .map(|row| {
            let emission_percentage = (row.total_emission_share * 100.0).min(100.0);
            MechanismSummary {
                id: format!("mechanism-{}", row.mechanism_id),
                name: format!("Mechanism {}", row.mechanism_id),
                emission_percentage,
                challenge_count: row.challenge_count as usize,
                status: if row.challenge_count > 0 {
                    "live".to_string()
                } else {
                    "coming-soon".to_string()
                },
            }
        })
        .collect();

    let total_emissions_percentage: f64 = mechanisms_summary
        .iter()
        .map(|m| m.emission_percentage)
        .sum();

    // Get validator counts (try to get from jobs table as proxy)
    // In a real implementation, this would query a validators table or WebSocket connections
    let active_validators: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(DISTINCT validator_hotkey) 
        FROM jobs 
        WHERE created_at > NOW() - INTERVAL '24 hours'
        "#,
    )
    .persistent(false)
    .fetch_one(pool.as_ref())
    .await
    .unwrap_or(0);

    let total_validators: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(DISTINCT validator_hotkey) 
        FROM jobs
        "#,
    )
    .persistent(false)
    .fetch_one(pool.as_ref())
    .await
    .unwrap_or(0);

    Ok(Json(NetworkOverviewResponse {
        total_challenges: total_challenges as usize,
        active_challenges: active_challenges as usize,
        total_validators: total_validators as usize,
        active_validators: active_validators as usize,
        total_jobs_processed: total_jobs_processed as usize,
        total_emissions_percentage,
        mechanisms_summary,
    }))
}
