use axum::{extract::State, http::StatusCode, response::Json, routing::get, Router};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;

use crate::state::AppState;

/// Create mechanisms router
pub fn create_router() -> Router<AppState> {
    Router::new().route("/mechanisms", get(list_mechanisms))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectInMechanism {
    pub id: String,
    pub name: String,
    pub status: String,
    pub description: String,
    pub tech_stack: Vec<String>,
    pub contributors: i32,
    pub stars: i32,
    pub last_updated: String,
    pub details: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MechanismResponse {
    pub id: String,
    pub name: String,
    pub emission_percentage: f64,
    pub status: String,
    pub challenges: Vec<ChallengeInMechanism>,
    pub projects: Vec<ProjectInMechanism>,
    pub total_challenges: usize,
    pub total_emission_share: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChallengeInMechanism {
    pub id: String,
    pub name: String,
    pub status: String,
    pub description: Option<String>,
    pub emission_share: f64,
    pub difficulty: Option<String>,
    pub github_repo: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct MechanismsListResponse {
    pub mechanisms: Vec<MechanismResponse>,
    pub total: usize,
}

/// List all mechanisms with their challenges
pub async fn list_mechanisms(
    State(state): State<AppState>,
) -> Result<Json<MechanismsListResponse>, StatusCode> {
    let pool = state.database_pool.as_ref().ok_or_else(|| {
        tracing::error!("Database pool not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Query challenges grouped by mechanism_id
    #[derive(sqlx::FromRow)]
    struct ChallengeRow {
        id: uuid::Uuid,
        name: String,
        mechanism_id: i16,
        emission_share: f64,
        description: Option<String>,
        github_repo: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    let challenges = sqlx::query_as::<_, ChallengeRow>(
        r#"
        SELECT 
            id, name, mechanism_id, emission_share, description, github_repo,
            created_at, updated_at
        FROM challenges
        ORDER BY mechanism_id, created_at DESC
        "#,
    )
    .persistent(false)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query challenges: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Group challenges by mechanism_id
    let mut mechanisms_map: HashMap<i16, Vec<ChallengeInMechanism>> = HashMap::new();

    for challenge in challenges {
        // Determine status based on challenge (for now, assume active if exists)
        // In the future, this could check validator_challenge_status table
        let status = "live".to_string();

        // Determine difficulty (could be based on emission_share or other factors)
        let difficulty = if challenge.emission_share > 0.5 {
            Some("Expert".to_string())
        } else if challenge.emission_share > 0.3 {
            Some("Hard".to_string())
        } else if challenge.emission_share > 0.1 {
            Some("Medium".to_string())
        } else {
            Some("Easy".to_string())
        };

        let challenge_data = ChallengeInMechanism {
            id: challenge.id.to_string(),
            name: challenge.name.clone(),
            status,
            description: challenge.description,
            emission_share: challenge.emission_share,
            difficulty,
            github_repo: challenge.github_repo,
            created_at: challenge.created_at.to_rfc3339(),
            updated_at: challenge.updated_at.to_rfc3339(),
        };

        mechanisms_map
            .entry(challenge.mechanism_id)
            .or_insert_with(Vec::new)
            .push(challenge_data);
    }

    // Convert to response format
    let mut mechanisms: Vec<MechanismResponse> = mechanisms_map
        .into_iter()
        .map(|(mechanism_id, challenges)| {
            let total_emission_share: f64 = challenges.iter().map(|c| c.emission_share).sum();
            
            // Calculate mechanism emission percentage (sum of all challenge emission shares)
            // This is a simplified calculation - in reality, mechanism emission might be configured separately
            let emission_percentage = (total_emission_share * 100.0).min(100.0);

            MechanismResponse {
                id: format!("mechanism-{}", mechanism_id),
                name: format!("Mechanism {}", mechanism_id),
                emission_percentage,
                status: if challenges.is_empty() {
                    "coming-soon".to_string()
                } else {
                    "live".to_string()
                },
                total_challenges: challenges.len(),
                total_emission_share,
                challenges,
                projects: Vec::new(), // Projects will be added when projects table is implemented
            }
        })
        .collect();

    // Sort by mechanism_id
    mechanisms.sort_by_key(|m| {
        m.id
            .strip_prefix("mechanism-")
            .and_then(|s| s.parse::<i16>().ok())
            .unwrap_or(0)
    });

    Ok(Json(MechanismsListResponse {
        total: mechanisms.len(),
        mechanisms,
    }))
}





