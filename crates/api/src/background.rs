use crate::state::AppState;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use platform_api_models::{ChallengePort, ChallengeResources, ChallengeSpec};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

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

        // IMPORTANT: Wait a bit for database to be fully ready
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Sync at startup - CRITICAL for initial load
        info!("Initial sync: Loading challenges from database...");
        match sync_challenges_from_db(&state, &pool).await {
            Ok(_) => {
                info!("Initial sync completed successfully");

                // Verify registry is populated
                let registry_size = {
                    let reg = state.challenge_registry.read().await;
                    reg.len()
                };
                info!(
                    "Registry now contains {} challenges after initial sync",
                    registry_size
                );
            }
            Err(e) => {
                error!("Failed to sync challenges from DB at startup: {}", e);
            }
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

/// Public function to sync challenges from database on demand
pub async fn sync_challenges_from_db_once(
    state: &AppState,
    pool: &Arc<PgPool>,
) -> anyhow::Result<()> {
    sync_challenges_from_db(state, pool).await
}

async fn sync_challenges_from_db(state: &AppState, pool: &PgPool) -> anyhow::Result<()> {
    info!("🔄 sync_challenges_from_db: Starting database synchronization");

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
        "#,
    )
    .persistent(false)
    .fetch_all(pool)
    .await?;

    info!("📊 Database query completed: {} rows returned", rows.len());

    if rows.is_empty() {
        warn!("⚠️ No challenges found in database! The challenges table is empty.");
        warn!("   Run: docker exec -i platform-postgres-dev psql -U platform -d platform_dev < docker-init-db/02-insert-term-challenge.sql");
    }

    let mut new_challenges = std::collections::HashMap::new();

    for (idx, row) in rows.iter().enumerate() {
        info!(
            "Processing row {}: name='{}', compose_hash='{}'",
            idx + 1,
            row.name,
            row.compose_hash
        );

        // Parse JSON fields with error handling
        let resources: ChallengeResources = match serde_json::from_value(row.resources.clone()) {
            Ok(r) => r,
            Err(e) => {
                error!(
                    "Failed to parse resources for challenge {}: {}",
                    row.name, e
                );
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
        let env: std::collections::BTreeMap<String, String> =
            match serde_json::from_value(row.env.clone()) {
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
            name: row.name.clone(),
            compose_hash: row.compose_hash.clone(),
            compose_yaml: compose_yaml_base64,
            version: row.version.clone(),
            images: row.images.clone(),
            resources,
            ports,
            env,
            emission_share: row.emission_share,
            mechanism_id: row.mechanism_id.max(0) as u8, // Convert i16 to u8, ensure non-negative
            weight: row.weight,
            description: row.description.clone(),
            mermaid_chart: row.mermaid_chart.clone(),
            github_repo: row.github_repo.clone(),
            dstack_image: row.dstack_image.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        };

        new_challenges.insert(row.compose_hash.clone(), challenge);
        debug!("Added challenge '{}' to new_challenges map", row.name);
    }

    // Log all challenges found in database
    debug!("Found {} challenges in database:", new_challenges.len());
    for (hash, challenge) in &new_challenges {
        info!(
            "  - {} (hash: {}) - version: {} - mechanism: {} - images: {:?}",
            challenge.name, hash, challenge.version, challenge.mechanism_id, challenge.images
        );
    }

    // Update the challenge registry - ALWAYS FORCE UPDATE
    debug!("Acquiring write lock on registry...");
    let mut registry = state.challenge_registry.write().await;
    let registry_len_before = registry.len();
    info!("Write lock acquired, current size: {}", registry_len_before);

    // Track which challenges are new or changed
    let mut new_or_changed = Vec::new();

    // CRITICAL: Clear and rebuild registry to ensure it's in sync
    if !new_challenges.is_empty() {
        info!(
            "🔄 Force-syncing {} challenges from PostgreSQL to memory",
            new_challenges.len()
        );

        // Track challenges that are truly new (not in registry before clearing)
        for hash in new_challenges.keys() {
            if !registry.contains_key(hash) {
                new_or_changed.push(hash.clone());
            }
        }

        // Clear the registry first
        registry.clear();
        info!("🧹 Registry cleared");

        // Add all challenges one by one with logging
        for (hash, challenge) in new_challenges.iter() {
            registry.insert(hash.clone(), challenge.clone());
            info!("➕ Added to registry: {} (hash: {})", challenge.name, hash);
        }

        // If we're doing a force-sync and no challenges were marked as new,
        // it means they were already in the registry. However, we should still
        // check if they're running and start them if not. For now, mark all as new/changed
        // during force-sync to ensure they're started.
        if new_or_changed.is_empty() {
            debug!("No new challenges detected, but force-sync detected. Checking if challenges need to be started...");
            // Add all challenges to new_or_changed to ensure they're started
            for hash in new_challenges.keys() {
                new_or_changed.push(hash.clone());
            }
        }

        info!(
            "Registry force-updated: now contains {} challenges (was {})",
            registry.len(),
            registry_len_before
        );

        // Verify the update succeeded
        if registry.is_empty() && !new_challenges.is_empty() {
            error!("CRITICAL: Registry is still empty after update! This should not happen.");
        }

        // Log final registry contents
        debug!("Final registry contents:");
        for (hash, challenge) in registry.iter() {
            debug!("  {} (hash: {})", challenge.name, hash);
        }
    } else if registry.is_empty() {
        warn!("No challenges found in database AND registry is empty!");
        warn!("   Make sure to run: docker exec -i platform-postgres-dev psql -U platform -d platform_dev < docker-init-db/02-insert-term-challenge.sql");
    } else {
        info!("Registry unchanged, contains {} challenges", registry.len());
    }

    drop(registry);

    // Auto-start new or changed challenges if ChallengeRunner is available
    // Only start challenges that we confirmed exist in the database
    if let Some(runner) = &state.challenge_runner {
        // Filter out challenges that are already running
        let mut challenges_to_start = Vec::new();
        let running_challenges = runner.list_running_challenges().await;
        let running_compose_hashes: std::collections::HashSet<String> = running_challenges
            .iter()
            .map(|c| c.compose_hash.clone())
            .collect();

        for compose_hash in new_or_changed {
            if !running_compose_hashes.contains(&compose_hash) {
                challenges_to_start.push(compose_hash);
            } else {
                info!(
                    compose_hash = &compose_hash,
                    "Challenge already running, skipping auto-start"
                );
            }
        }

        for compose_hash in challenges_to_start {
            // Verify the challenge still exists in new_challenges (which came from DB)
            if let Some(challenge_spec) = new_challenges.get(&compose_hash) {
                info!(
                    compose_hash = &compose_hash,
                    "Auto-starting challenge with compose_hash"
                );
                // Use the challenge_spec we already have to avoid re-querying
                // Note: Errors here are logged but don't prevent the challenge from being marked as failed
                // Some errors (like migration timeouts, schema check failures) are non-fatal

                // Load environment variables for this challenge
                let env_vars = match state.load_challenge_env_vars(&compose_hash).await {
                    Ok(vars) => {
                        if !vars.is_empty() {
                            info!(
                                compose_hash = &compose_hash,
                                count = vars.len(),
                                "Loaded {} environment variables for challenge",
                                vars.len()
                            );
                        }
                        Some(vars)
                    }
                    Err(e) => {
                        warn!(
                            compose_hash = &compose_hash,
                            error = %e,
                            "Failed to load environment variables for challenge (continuing without them)"
                        );
                        None
                    }
                };

                match runner
                    .run_challenge_by_compose_hash_with_spec(
                        &compose_hash,
                        Some(challenge_spec),
                        env_vars,
                    )
                    .await
                {
                    Ok(_) => {
                        info!(
                            compose_hash = &compose_hash,
                            "Challenge auto-started successfully"
                        );
                    }
                    Err(e) => {
                        let error_str = e.to_string();
                        // Only log as error if it's a fatal error (CVM deployment failure, etc.)
                        // Non-fatal errors (migrations timeout, schema check) are logged as warnings
                        if error_str.contains("Failed to check existing schema")
                            || error_str.contains("Timeout waiting for db_version")
                            || error_str.contains("Failed to get migrations")
                            || error_str.contains("Failed to apply migrations")
                        {
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

        // Refresh every 60 seconds (matching METAGRAPH_CACHE_TTL_SEC from terminal-challenge)
        let mut interval = interval(Duration::from_secs(60));

        loop {
            interval.tick().await;
            refresh_metagraph_cache().await;
        }
    });
}
