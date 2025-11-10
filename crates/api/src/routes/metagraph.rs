use axum::{extract::State, http::StatusCode, response::Json, routing::get, Router};
use serde_json::json;
use std::collections::HashSet;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::state::AppState;

/// Metagraph cache (in-memory)
static METAGRAPH_CACHE: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();

pub fn get_metagraph_cache() -> &'static RwLock<HashSet<String>> {
    METAGRAPH_CACHE.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Get netuid from environment or use default subnet (100)
fn get_netuid() -> u16 {
    std::env::var("BT_NETUID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100) // Default to subnet 100, can be overridden via BT_NETUID env var
}

/// Create metagraph router
pub fn create_router() -> Router<AppState> {
    Router::new().route("/api/metagraph/hotkeys", get(get_metagraph_hotkeys))
}

/// Get list of valid hotkeys from metagraph cache
///
/// Returns JSON with list of hotkeys in ss58 format.
/// This endpoint is used by coding-benchmark to verify miner hotkeys.
pub async fn get_metagraph_hotkeys(
    State(_state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let cache = get_metagraph_cache();
    let cache_guard = cache.read().await;
    let hotkeys: Vec<String> = cache_guard.iter().cloned().collect();

    Ok(Json(json!({
        "hotkeys": hotkeys,
        "count": hotkeys.len(),
        "cache_timestamp": chrono::Utc::now().to_rfc3339(),
    })))
}

/// Initialize metagraph cache by syncing from Bittensor chain/subtensor
pub async fn refresh_metagraph_cache() {
    let cache = get_metagraph_cache();
    let netuid = get_netuid();

    info!(
        netuid = netuid,
        "Starting metagraph cache refresh from Bittensor chain"
    );

    match sync_metagraph_from_chain(netuid).await {
        Ok(hotkeys) => {
            let mut cache_guard = cache.write().await;
            *cache_guard = hotkeys;
            info!(
                netuid = netuid,
                hotkey_count = cache_guard.len(),
                "Metagraph cache refreshed successfully"
            );
        }
        Err(e) => {
            error!(
                netuid = netuid,
                error = %e,
                "Failed to refresh metagraph cache from chain"
            );
            // Don't clear cache on error, keep existing data
        }
    }
}

/// Sync metagraph from Bittensor chain and extract all hotkeys
async fn sync_metagraph_from_chain(netuid: u16) -> anyhow::Result<HashSet<String>> {
    use bittensor_rs::chain::BittensorClient;
    use bittensor_rs::queries::neurons;
    use bittensor_rs::utils::ss58::encode_ss58;

    // Create Bittensor client with default connection (connects to mainnet finney)
    // Can be overridden via BT_ENDPOINT env var in BittensorClient::with_default()
    let client = BittensorClient::with_default()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create Bittensor client: {}", e))?;

    info!(netuid = netuid, "Querying neurons from Bittensor chain");

    // Get all neurons for the subnet
    let neurons_list = neurons::neurons(&client, netuid, None)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to query neurons: {}", e))?;

    info!(
        netuid = netuid,
        neuron_count = neurons_list.len(),
        "Retrieved neurons from chain"
    );

    // Extract hotkeys and convert to ss58 format
    let mut hotkeys = HashSet::new();
    for neuron in neurons_list {
        let hotkey_ss58 = encode_ss58(&neuron.hotkey);
        hotkeys.insert(hotkey_ss58);
    }

    Ok(hotkeys)
}
