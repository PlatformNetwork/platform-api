use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::{info, error, warn};
use sqlx::PgPool;
use chrono::{DateTime, Utc};
use platform_api_models::{ChallengeSpec, ChallengeResources, ChallengePort};
use crate::state::AppState;
use serde_json::Value;
use base64::{Engine as _, engine::general_purpose};

/// Start background task to sync challenges from PostgreSQL
pub fn start_challenge_sync_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        info!("Starting challenge sync task - reading from PostgreSQL every 1 minute");
        
        // Get existing database pool from AppState
        let pool = match &state.database_pool {
            Some(pool) => pool.clone(),
            None => {
                error!("No database pool available - PostgreSQL storage not configured");
                return;
            }
        };
        
        // Sync at startup
        info!("🔄 Initial sync: Loading challenges from database...");
        if let Err(e) = sync_challenges_from_db(&state, &pool).await {
            error!("Failed to sync challenges from DB at startup: {}", e);
        }
        
        let mut interval = interval(Duration::from_secs(60));
        
        loop {
            interval.tick().await;
            
            if let Err(e) = sync_challenges_from_db(&state, &pool).await {
                error!("Failed to sync challenges from DB: {}", e);
            }
        }
    });
}

async fn sync_challenges_from_db(state: &AppState, pool: &PgPool) -> anyhow::Result<()> {
    // Read challenges from PostgreSQL using query_as to avoid prepared statement conflicts
    #[derive(sqlx::FromRow)]
    struct ChallengeRow {
        id: uuid::Uuid,
        name: String,
        compose_hash: String,
        compose_yaml: String,
        version: String,
        images: Vec<String>,
        resources: Value,
        ports: Value,
        env: Value,
        emission_share: f64,
        mechanism_id: i16, // PostgreSQL SMALLINT maps to i16, will convert to u8
        weight: Option<f64>,
        description: Option<String>,
        mermaid_chart: Option<String>,
        github_repo: Option<String>,
        dstack_image: Option<String>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    }
    
    let rows = sqlx::query_as::<_, ChallengeRow>(
        r#"
        SELECT 
            id, name, compose_hash, compose_yaml, version, images,
            resources, ports, env, emission_share, mechanism_id, weight,
            description, mermaid_chart, github_repo, dstack_image, created_at, updated_at
        FROM challenges
        "#
    )
    .persistent(false)
    .fetch_all(pool)
    .await?;
    
    let mut new_challenges = std::collections::HashMap::new();
    
    for row in rows {
        // Parse JSON fields with error handling
        let resources: ChallengeResources = match serde_json::from_value(row.resources.clone()) {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to parse resources for challenge {}: {}", row.name, e);
                error!("Resources JSON: {:?}", row.resources);
                continue; // Skip this challenge if parsing fails
            }
        };
        let ports: Vec<ChallengePort> = match serde_json::from_value(row.ports.clone()) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to parse ports for challenge {}: {}", row.name, e);
                error!("Ports JSON: {:?}", row.ports);
                continue;
            }
        };
        let env: std::collections::BTreeMap<String, String> = match serde_json::from_value(row.env.clone()) {
            Ok(e) => e,
            Err(e) => {
                error!("Failed to parse env for challenge {}: {}", row.name, e);
                error!("Env JSON: {:?}", row.env);
                continue;
            }
        };
        
        // Encode compose_yaml to base64 (validator expects base64-encoded compose_yaml)
        let compose_yaml_base64 = general_purpose::STANDARD.encode(&row.compose_yaml);
        
        let challenge = ChallengeSpec {
            id: row.id,
            name: row.name,
            compose_hash: row.compose_hash.clone(),
            compose_yaml: compose_yaml_base64,
            version: row.version,
            images: row.images,
            resources,
            ports,
            env,
            emission_share: row.emission_share,
            mechanism_id: row.mechanism_id.max(0) as u8, // Convert i16 to u8, ensure non-negative
            weight: row.weight,
            description: row.description,
            mermaid_chart: row.mermaid_chart,
            github_repo: row.github_repo,
            dstack_image: row.dstack_image,
            created_at: row.created_at,
            updated_at: row.updated_at,
        };
        
        new_challenges.insert(row.compose_hash, challenge);
    }
    
    // Log all challenges found in database
    info!("📊 Found {} challenges in database:", new_challenges.len());
    for (hash, challenge) in &new_challenges {
        info!("  - {} (hash: {}) - version: {} - mechanism: {} - images: {:?}", 
            challenge.name, hash, challenge.version, challenge.mechanism_id, challenge.images);
    }
    
    // Update the challenge registry
    let mut registry = state.challenge_registry.write().await;
    
    // Track which challenges are new or changed (check before updating registry)
    let mut new_or_changed = Vec::new();
    for (hash, _) in &new_challenges {
        if !registry.contains_key(hash) {
            new_or_changed.push(hash.clone());
        }
    }
    
    // Check if there are changes
    let registry_len_before = registry.len();
    let has_changes = registry.len() != new_challenges.len() || 
                      registry.iter().any(|(hash, _)| !new_challenges.contains_key(hash));
    
    info!("🔍 Registry check: before={}, new={}, has_changes={}", registry_len_before, new_challenges.len(), has_changes);
    
    if has_changes {
        info!("🔄 Syncing {} challenges from PostgreSQL to memory", new_challenges.len());
        *registry = new_challenges.clone();
        info!("✅ Registry updated: {} challenges now in memory (was {})", registry.len(), registry_len_before);
    } else {
        info!("📋 No changes detected: {} challenges already in registry", registry.len());
    }
    
    drop(registry);
    
    // Auto-start new or changed challenges if ChallengeRunner is available
    // Only start challenges that we confirmed exist in the database
    if let Some(runner) = &state.challenge_runner {
        for compose_hash in new_or_changed {
            // Verify the challenge still exists in new_challenges (which came from DB)
            if let Some(challenge_spec) = new_challenges.get(&compose_hash) {
                info!(
                    compose_hash = &compose_hash,
                    "Auto-starting challenge with compose_hash"
                );
                // Use the challenge_spec we already have to avoid re-querying
                // Note: Errors here are logged but don't prevent the challenge from being marked as failed
                // Some errors (like migration timeouts, schema check failures) are non-fatal
                match runner.run_challenge_by_compose_hash_with_spec(&compose_hash, Some(challenge_spec)).await {
                    Ok(_) => {
                        info!(
                            compose_hash = &compose_hash,
                            "✅ Challenge auto-started successfully"
                        );
                    }
                    Err(e) => {
                        let error_str = e.to_string();
                        // Only log as error if it's a fatal error (CVM deployment failure, etc.)
                        // Non-fatal errors (migrations timeout, schema check) are logged as warnings
                        if error_str.contains("Failed to check existing schema") 
                            || error_str.contains("Timeout waiting for db_version")
                            || error_str.contains("Failed to get migrations")
                            || error_str.contains("Failed to apply migrations") {
                            warn!(
                                compose_hash = &compose_hash,
                                error = %e,
                                "Non-fatal error during challenge start (challenge will continue)"
                            );
                        } else {
                            error!(
                                compose_hash = &compose_hash,
                                error = %e,
                                "Failed to auto-start challenge"
                            );
                        }
                    }
                }
            } else {
                warn!(
                    compose_hash = &compose_hash,
                    "Challenge compose_hash not in database, skipping auto-start"
                );
            }
        }
    }
    
    Ok(())
}

/// Start background task to sync metagraph hotkeys from Bittensor chain
pub fn start_metagraph_sync_task() {
    tokio::spawn(async move {
        use crate::routes::metagraph::refresh_metagraph_cache;
        
        info!("Starting metagraph sync task - refreshing from Bittensor chain every 60 seconds");
        
        // Initial sync at startup
        info!("🔄 Initial metagraph sync: Loading hotkeys from Bittensor chain...");
        refresh_metagraph_cache().await;
        
        // Refresh every 60 seconds (matching METAGRAPH_CACHE_TTL_SEC from coding-benchmark)
        let mut interval = interval(Duration::from_secs(60));
        
        loop {
            interval.tick().await;
            refresh_metagraph_cache().await;
        }
    });
}

