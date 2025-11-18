use anyhow::Result;
use platform_api_models::*;
use reqwest::Client;
use std::collections::HashMap;
use tracing::{error, info};

/// Scoring service for triggering challenge scoring runs
pub struct ScoringService {
    client: Client,
    last_scored_block: tokio::sync::RwLock<u64>,
    challenge_registry: Arc<tokio::sync::RwLock<HashMap<String, ChallengeSpec>>>,
}

impl ScoringService {
    pub fn new(
        challenge_registry: Arc<tokio::sync::RwLock<HashMap<String, ChallengeSpec>>>,
    ) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap(),
            last_scored_block: tokio::sync::RwLock::new(0),
            challenge_registry,
        }
    }

    /// Check if scoring should be triggered (every 360 blocks)
    pub async fn should_trigger_scoring(&self, current_block: u64) -> bool {
        let last_block = *self.last_scored_block.read().await;
        current_block >= last_block + 360
    }

    /// Trigger scoring for all active challenges
    pub async fn trigger_scoring(&self, current_block: u64) -> Result<Vec<ScoringResult>> {
        info!("Triggering scoring run at block {}", current_block);

        let challenges = self.challenge_registry.read().await;
        let mut results = Vec::new();

        for (compose_hash, spec) in challenges.iter() {
            match self.score_challenge(compose_hash, spec).await {
                Ok(result) => {
                    info!(
                        "Scoring completed for challenge {}: score={}",
                        compose_hash, result.score
                    );
                    results.push(result);
                }
                Err(e) => {
                    error!("Scoring failed for challenge {}: {}", compose_hash, e);
                    results.push(ScoringResult {
                        compose_hash: compose_hash.clone(),
                        score: 0.0,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        // Update last scored block
        let mut last_block = self.last_scored_block.write().await;
        *last_block = current_block;

        Ok(results)
    }

    /// Score a single challenge
    async fn score_challenge(
        &self,
        compose_hash: &str,
        spec: &ChallengeSpec,
    ) -> Result<ScoringResult> {
        // Construct challenge API URL
        let challenge_url = format!("https://challenge-{}:10000", compose_hash);

        // Try to contact challenge API with 60s timeout
        match self
            .client
            .get(format!("{}/score", challenge_url))
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    let score_data: serde_json::Value = response.json().await?;
                    let score = score_data["score"].as_f64().unwrap_or(0.0);

                    Ok(ScoringResult {
                        compose_hash: compose_hash.to_string(),
                        score,
                        error: None,
                    })
                } else {
                    Err(anyhow::anyhow!(
                        "Challenge returned error status: {}",
                        response.status()
                    ))
                }
            }
            Err(e) => Err(anyhow::anyhow!("Failed to reach challenge API: {}", e)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScoringResult {
    pub compose_hash: String,
    pub score: f64,
    pub error: Option<String>,
}

use std::sync::Arc;
