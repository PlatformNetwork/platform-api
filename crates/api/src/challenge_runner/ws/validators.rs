//! Validator management utilities

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Get list of active validator hotkeys for a specific compose_hash
/// Uses both validator_challenge_status (if available) and validator_connections (WebSocket connections)
pub async fn get_active_validators_for_compose_hash(
    compose_hash: &str,
    validator_status: &Arc<
        RwLock<
            std::collections::HashMap<
                String,
                std::collections::HashMap<
                    String,
                    platform_api_models::ValidatorChallengeStatus,
                >,
            >,
        >,
    >,
    validator_connections: &Option<
        Arc<
            RwLock<
                std::collections::HashMap<String, crate::state::ValidatorConnection>,
            >,
        >,
    >,
) -> Vec<String> {
    let mut validators = HashSet::new();

    // First, get validators from validator_challenge_status (if they have Active state)
    let status_map = validator_status.read().await;
    for (hotkey, challenge_statuses) in status_map.iter() {
        if let Some(status) = challenge_statuses.get(compose_hash) {
            if matches!(
                status.state,
                platform_api_models::ValidatorChallengeState::Active
            ) {
                validators.insert(hotkey.clone());
            }
        }
    }
    drop(status_map);

    // Also include validators connected via WebSocket with matching compose_hash
    // This is a fallback when validator_challenge_status doesn't have Active state
    if let Some(validator_conns) = validator_connections {
        let connections = validator_conns.read().await;
        for (hotkey, conn) in connections.iter() {
            if conn.compose_hash.as_deref() == Some(compose_hash) {
                validators.insert(hotkey.clone());
            }
        }
    }

    validators.into_iter().collect()
}
