use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Json, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use hex;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx;
use std::collections::HashMap;
use uuid::Uuid;

use crate::state::AppState;
use platform_api_models;

#[derive(Debug, Serialize, Deserialize)]
struct RenderLinkClaims {
    sub: String,
    jti: String,
    aud: String,
    exp: usize,
    iat: usize,
    reviewer_id: String,
    challenge_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionClaims {
    sub: String,
    jti: String,
    aud: String,
    exp: usize,
    iat: usize,
    reviewer_id: String,
    challenge_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRenderLinkRequest {
    pub reviewer_id: String,
    pub challenge_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct CreateRenderLinkResponse {
    pub link_token: String,
    pub render_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ExchangeSessionRequest {
    pub link_token: String,
}

#[derive(Debug, Serialize)]
pub struct ExchangeSessionResponse {
    pub session_id: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ChallengeReviewData {
    pub challenge_id: Uuid,
    pub challenge_name: String,
    pub submissions: Vec<SubmissionReview>,
    pub stats: ReviewStats,
}

#[derive(Debug, Serialize)]
pub struct SubmissionReview {
    pub submission_id: Uuid,
    pub miner_hotkey: String,
    pub score: Option<f64>,
    pub components: HashMap<String, f64>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub proof: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReviewStats {
    pub total_submissions: u64,
    pub pending: u64,
    pub reviewed: u64,
    pub average_score: f64,
}

#[derive(Debug, Deserialize)]
pub struct SubmitDecisionRequest {
    pub submission_id: Uuid,
    pub decision: String,
    pub score: Option<f64>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SubmitDecisionResponse {
    pub decision_id: Uuid,
    pub receipt: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct SubmissionProofs {
    pub submission_id: Uuid,
    pub proofs: Vec<ProofReceipt>,
}

#[derive(Debug, Serialize)]
pub struct ProofReceipt {
    pub proof: String,
    pub signature: String,
    pub timestamp: DateTime<Utc>,
    pub validator: String,
    pub score_hash: String,
}

#[derive(Debug, Serialize)]
pub struct JobListItem {
    pub job_id: Uuid,
    pub submission_id: Uuid,
    pub miner_hotkey: String,
    pub status: String,
    pub priority: String,
    pub created_at: DateTime<Utc>,
}

pub fn create_router() -> Router<AppState> {
    Router::new()
        .route("/ui/render/link", post(create_render_link))
        .route("/ui/render/session", post(exchange_session))
        .route(
            "/ui/challenges/:id/review-data",
            get(get_challenge_review_data),
        )
        .route("/ui/challenges/:id/decision", post(submit_decision))
        .route("/ui/submissions/:id/proofs", get(get_submission_proofs))
        .route("/ui/jobs", get(list_jobs_for_ui))
}

pub async fn create_render_link(
    State(state): State<AppState>,
    Json(request): Json<CreateRenderLinkRequest>,
) -> Result<Json<CreateRenderLinkResponse>, StatusCode> {
    // Check if JWT is disabled
    if state.config.jwt_secret_ui == "disabled-no-jwt" || state.config.jwt_secret_ui.is_empty() {
        return Err(StatusCode::NOT_IMPLEMENTED);
    }

    let jti = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + Duration::minutes(5);

    let secret = state.config.jwt_secret_ui.clone();
    let signing_key = EncodingKey::from_secret(secret.as_bytes());

    let claims = RenderLinkClaims {
        sub: request.reviewer_id.clone(),
        jti: jti.clone(),
        aud: "challenge-render".to_string(),
        exp: expires_at.timestamp() as usize,
        iat: Utc::now().timestamp() as usize,
        reviewer_id: request.reviewer_id.clone(),
        challenge_id: request.challenge_id.to_string(),
    };

    let token = encode(&Header::new(Algorithm::HS256), &claims, &signing_key)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let render_url = format!("https://challenge-render.platform.ai?token={}", token);

    Ok(Json(CreateRenderLinkResponse {
        link_token: token,
        render_url,
        expires_at,
    }))
}

pub async fn exchange_session(
    State(state): State<AppState>,
    _headers: HeaderMap,
    Json(request): Json<ExchangeSessionRequest>,
) -> Result<Response, StatusCode> {
    // Check if JWT is disabled
    if state.config.jwt_secret_ui == "disabled-no-jwt" || state.config.jwt_secret_ui.is_empty() {
        return Err(StatusCode::NOT_IMPLEMENTED);
    }

    let secret = state.config.jwt_secret_ui.clone();
    let decoding_key = DecodingKey::from_secret(secret.as_bytes());

    let token_data = decode::<RenderLinkClaims>(
        &request.link_token,
        &decoding_key,
        &Validation::new(Algorithm::HS256),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let claims = token_data.claims;

    let session_id = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + Duration::minutes(15);

    let session_claims = SessionClaims {
        sub: claims.reviewer_id.clone(),
        jti: session_id.clone(),
        aud: "challenge-render".to_string(),
        exp: expires_at.timestamp() as usize,
        iat: Utc::now().timestamp() as usize,
        reviewer_id: claims.reviewer_id,
        challenge_id: claims.challenge_id,
    };

    let signing_key = EncodingKey::from_secret(secret.as_bytes());
    let session_token = encode(
        &Header::new(Algorithm::HS256),
        &session_claims,
        &signing_key,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = ExchangeSessionResponse {
        session_id,
        expires_at,
    };

    let cookie_value = format!(
        "ui_session={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=900",
        session_token
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("Set-Cookie", cookie_value)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&response).unwrap().into())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn get_challenge_review_data(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ChallengeReviewData>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Get challenge name from database
    let challenge_name: String = sqlx::query_scalar("SELECT name FROM challenges WHERE id = $1")
        .persistent(false)
        .bind(id)
        .fetch_optional(pool.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to query challenge: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get submissions from challenge_results table
    #[derive(sqlx::FromRow)]
    struct ChallengeResultRow {
        id: uuid::Uuid,
        artifact_id: String, // This represents the miner/submission
        score: f64,
        weight: f64,
        justification: String,
        execution_time_ms: Option<i64>,
        error_message: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
    }

    let result_rows = sqlx::query_as::<_, ChallengeResultRow>(
        r#"
        SELECT id, artifact_id, score, weight, justification, 
               execution_time_ms, error_message, created_at
        FROM challenge_results
        WHERE compose_hash IN (SELECT compose_hash FROM challenges WHERE id = $1)
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .persistent(false)
    .bind(id)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query challenge results: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Convert to SubmissionReview
    let submissions: Vec<SubmissionReview> = result_rows
        .into_iter()
        .map(|row| {
            // Parse components from justification or use score as default
            let mut components = HashMap::new();
            components.insert("score".to_string(), row.score);
            components.insert("weight".to_string(), row.weight);

            SubmissionReview {
                submission_id: row.id,
                miner_hotkey: row.artifact_id.chars().take(10).collect::<String>() + "...", // Truncate for display
                score: Some(row.score),
                components,
                status: if row.error_message.is_some() {
                    "failed".to_string()
                } else {
                    "completed".to_string()
                },
                created_at: row.created_at,
                proof: Some(format!("result:{}", row.id)), // Use result ID as proof reference
            }
        })
        .collect();

    // Calculate stats
    let total_submissions = submissions.len() as u64;
    let pending = submissions.iter().filter(|s| s.status == "pending").count() as u64;
    let reviewed = submissions
        .iter()
        .filter(|s| s.status == "completed" || s.status == "failed")
        .count() as u64;
    let average_score = if !submissions.is_empty() {
        submissions.iter().filter_map(|s| s.score).sum::<f64>() / submissions.len() as f64
    } else {
        0.0
    };

    let stats = ReviewStats {
        total_submissions,
        pending,
        reviewed,
        average_score,
    };

    Ok(Json(ChallengeReviewData {
        challenge_id: id,
        challenge_name,
        submissions,
        stats,
    }))
}

pub async fn submit_decision(
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
    Json(_request): Json<SubmitDecisionRequest>,
) -> Result<Json<SubmitDecisionResponse>, StatusCode> {
    let decision_id = Uuid::new_v4();
    let receipt = format!("decision:{}:{}", decision_id, Utc::now());

    Ok(Json(SubmitDecisionResponse {
        decision_id,
        receipt,
        timestamp: Utc::now(),
    }))
}

pub async fn get_submission_proofs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionProofs>, StatusCode> {
    let pool = state
        .database_pool
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Get proofs from challenge_results table
    #[derive(sqlx::FromRow)]
    struct ProofRow {
        id: uuid::Uuid,
        validator_hotkey: String,
        score: f64,
        justification: String,
        created_at: chrono::DateTime<chrono::Utc>,
    }

    let proof_rows = sqlx::query_as::<_, ProofRow>(
        r#"
        SELECT id, validator_hotkey, score, justification, created_at
        FROM challenge_results
        WHERE id = $1 OR artifact_id = $1::text
        ORDER BY created_at DESC
        "#,
    )
    .persistent(false)
    .bind(id)
    .fetch_all(pool.as_ref())
    .await
    .map_err(|e| {
        tracing::error!("Failed to query submission proofs: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Convert to ProofReceipt
    let proofs: Vec<ProofReceipt> = proof_rows
        .into_iter()
        .map(|row| {
            // Generate score hash from score and justification
            let mut hasher = Sha256::new();
            hasher.update(row.score.to_string().as_bytes());
            hasher.update(row.justification.as_bytes());
            let score_hash = hex::encode(hasher.finalize());

            ProofReceipt {
                proof: format!("result:{}", row.id), // Use result ID as proof
                signature: format!("validator:{}", row.validator_hotkey), // Simplified signature
                timestamp: row.created_at,
                validator: row.validator_hotkey,
                score_hash,
            }
        })
        .collect();

    Ok(Json(SubmissionProofs {
        submission_id: id,
        proofs,
    }))
}

pub async fn list_jobs_for_ui(
    State(state): State<AppState>,
    Query(params): Query<ListJobsForUIParams>,
) -> Result<Json<Vec<JobListItem>>, StatusCode> {
    // Use scheduler.list_jobs() which already uses PostgreSQL
    let jobs_response = state
        .scheduler
        .list_jobs(
            1,
            100,  // Get up to 100 jobs for UI
            None, // No status filter
            params.challenge_id,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to list jobs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Helper function to convert JobStatus to string
    fn status_to_string(status: &platform_api_models::JobStatus) -> String {
        match status {
            platform_api_models::JobStatus::Pending => "pending".to_string(),
            platform_api_models::JobStatus::Claimed => "claimed".to_string(),
            platform_api_models::JobStatus::Running => "running".to_string(),
            platform_api_models::JobStatus::Completed => "completed".to_string(),
            platform_api_models::JobStatus::Failed => "failed".to_string(),
            platform_api_models::JobStatus::Timeout => "timeout".to_string(),
        }
    }

    // Helper function to convert JobPriority to string
    fn priority_to_string(priority: &platform_api_models::JobPriority) -> String {
        match priority {
            platform_api_models::JobPriority::Low => "low".to_string(),
            platform_api_models::JobPriority::Normal => "normal".to_string(),
            platform_api_models::JobPriority::High => "high".to_string(),
            platform_api_models::JobPriority::Critical => "critical".to_string(),
        }
    }

    // Convert JobMetadata to JobListItem
    let jobs: Vec<JobListItem> = jobs_response
        .jobs
        .into_iter()
        .map(|job| {
            // Miner hotkey is not directly available in JobMetadata
            // For now, use "unknown" or we could query the database if needed
            let miner_hotkey = "unknown".to_string();

            // Use job.id as submission_id (they might be the same in some cases)
            let submission_id = job.id;

            JobListItem {
                job_id: job.id,
                submission_id,
                miner_hotkey,
                status: status_to_string(&job.status),
                priority: priority_to_string(&job.priority),
                created_at: job.created_at,
            }
        })
        .collect();

    Ok(Json(jobs))
}

#[derive(Debug, Deserialize)]
pub struct ListJobsForUIParams {
    pub challenge_id: Option<Uuid>,
}
