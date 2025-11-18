use anyhow::Context;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};
use uuid::Uuid;

use platform_api_orm_gateway::{ORMQuery, SecureORMGateway, TablePermission};
use crate::state::AppState;
use std::collections::HashMap;

/// Handle ORM query request
pub async fn handle_orm_query(
    orm_gateway: &Arc<RwLock<SecureORMGateway>>,
    query_data: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    // Parse the query
    let query: ORMQuery =
        serde_json::from_value(query_data.clone()).context("Failed to parse ORM query")?;

    // Execute the query
    let gateway = orm_gateway.read().await;
    let result = gateway
        .execute_read_query(query)
        .await
        .context("Failed to execute ORM query")?;

    // Convert result to JSON
    Ok(serde_json::to_value(result)?)
}

/// Handle ORM query from validator with challenge schema resolution
pub async fn handle_orm_query_with_challenge(
    state: &AppState,
    orm_gateway: &Arc<RwLock<SecureORMGateway>>,
    query_data: &serde_json::Value,
    challenge_id: &str,
    validator_hotkey: &str,
) -> anyhow::Result<serde_json::Value> {
    // Parse the query
    let mut query: ORMQuery =
        serde_json::from_value(query_data.clone()).context("Failed to parse ORM query")?;

    // Resolve schema: {challenge_name}_v{db_version}
    if query.schema.is_none() {
        // Get challenge_name from database using challenge_id
        let challenge_name = if let Some(pool) = &state.database_pool {
            // Try to parse challenge_id as UUID first
            let challenge_uuid = if let Ok(uuid) = Uuid::parse_str(challenge_id) {
                uuid
            } else {
                // If not a UUID, treat it as challenge name directly
                // Normalize challenge name (replace hyphens with underscores for schema)
                let schema_name = challenge_id.replace('-', "_");
                let db_version = query.db_version.unwrap_or(1);
                query.schema = Some(format!("{}_v{}", schema_name, db_version));
                info!(
                    validator_hotkey = validator_hotkey,
                    challenge_id = challenge_id,
                    schema = &query.schema.as_ref().unwrap(),
                    "Resolved schema from challenge_id (non-UUID) and db_version"
                );
                // Execute the query using normal gateway (permissions control access)
                let gateway = orm_gateway.read().await;
                let result = gateway
                    .execute_query(query)
                    .await
                    .context("Failed to execute ORM query")?;
                return Ok(serde_json::to_value(result)?);
            };

            // Query database for challenge name
            let name_result: Option<String> =
                sqlx::query_scalar("SELECT name FROM challenges WHERE id = $1 LIMIT 1")
                    .bind(challenge_uuid)
                    .fetch_optional(pool.as_ref())
                    .await
                    .context("Failed to query challenge name from database")?;

            match name_result {
                Some(name) => name,
                None => {
                    error!(
                        validator_hotkey = validator_hotkey,
                        challenge_id = challenge_id,
                        "Challenge not found in database"
                    );
                    return Err(anyhow::anyhow!("Challenge not found: {}", challenge_id));
                }
            }
        } else {
            error!(
                validator_hotkey = validator_hotkey,
                challenge_id = challenge_id,
                "Database pool not available"
            );
            return Err(anyhow::anyhow!("Database pool not available"));
        };

        // Get db_version from query, default to 1 if not provided
        let db_version = query.db_version.unwrap_or(1);

        // Normalize challenge name (replace hyphens with underscores for schema)
        let normalized_name = challenge_name.replace('-', "_");

        // Construct schema: {challenge_name}_v{db_version}
        query.schema = Some(format!("{}_v{}", normalized_name, db_version));

        info!(
            validator_hotkey = validator_hotkey,
            challenge_id = challenge_id,
            challenge_name = &challenge_name,
            db_version = db_version,
            schema = &query.schema.as_ref().unwrap(),
            "Resolved schema from challenge_name and db_version"
        );
    }

    // Execute the query using normal gateway (permissions control access)
    let gateway = orm_gateway.read().await;
    let result = gateway
        .execute_query(query)
        .await
        .context("Failed to execute ORM query")?;

    // Convert result to JSON
    Ok(serde_json::to_value(result)?)
}

/// Handle ORM permissions update
pub async fn handle_orm_permissions(
    orm_gateway: &Arc<RwLock<SecureORMGateway>>,
    challenge_id: &str,
    permissions_data: &serde_json::Value,
) -> anyhow::Result<()> {
    // Parse permissions
    let permissions: HashMap<String, TablePermission> =
        serde_json::from_value(permissions_data.clone())
            .context("Failed to parse ORM permissions")?;

    // Update permissions
    let mut gateway = orm_gateway.write().await;
    gateway
        .load_challenge_permissions(challenge_id, permissions)
        .await
        .context("Failed to load challenge permissions")?;

    Ok(())
}
