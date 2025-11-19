//! Refactored WebSocket handler for validator connections
//! Organized into modular components for better maintainability

use axum::{
    extract::{ws::WebSocketUpgrade, Path, State},
    response::Response,
};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::state::AppState;

use super::connection_manager::{handle_validator_connection, spawn_health_check_task, shutdown_connections};
use super::authentication::is_dev_mode;

/// WebSocket handler for validator connections
/// Entry point for all validator WebSocket connections
pub async fn validator_websocket(
    Path(hotkey): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    info!("Validator WebSocket connection request from: {}", hotkey);

    // Validate hotkey format
    if let Err(e) = validate_hotkey(&hotkey) {
        error!("Invalid hotkey format: {} - {}", hotkey, e);
        return axum::response::Json(serde_json::json!({
            "error": "Invalid hotkey format",
            "message": e.to_string()
        }))
        .into_response();
    }

    // Check if validator is registered and active
    if let Err(e) = validate_validator(&hotkey, &state).await {
        error!("Validator validation failed: {} - {}", hotkey, e);
        return axum::response::Json(serde_json::json!({
            "error": "Validator validation failed",
            "message": e.to_string()
        }))
        .into_response();
    }

    // Upgrade WebSocket connection with improved configuration
    ws.protocols(["platform-api-v1"])
        .max_frame_size(1024 * 1024) // 1MB max frame size
        .max_send_queue_size(100) // Limit send queue size
        .on_upgrade(move |socket| {
            handle_validator_connection(socket, hotkey, state)
        })
}

/// Validate hotkey format
fn validate_hotkey(hotkey: &str) -> Result<(), anyhow::Error> {
    if hotkey.is_empty() {
        return Err(anyhow::anyhow!("Hotkey cannot be empty"));
    }

    if hotkey.len() > 64 {
        return Err(anyhow::anyhow!("Hotkey too long (max 64 characters)"));
    }

    // Basic format validation - adjust according to your hotkey format
    if !hotkey.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(anyhow::anyhow!("Hotkey contains invalid characters"));
    }

    Ok(())
}

/// Validate that validator exists and is in good standing
async fn validate_validator(hotkey: &str, state: &AppState) -> Result<(), anyhow::Error> {
    // Check if validator exists
    let validator = state
        .get_validator_by_hotkey(hotkey)
        .await
        .map_err(|e| anyhow::anyhow!("Validator lookup failed: {}", e))?;

    // Check validator status
    if !validator.is_active {
        return Err(anyhow::anyhow!("Validator is not active"));
    }

    // Check if validator is already connected
    if state.is_validator_connected(hotkey).await {
        warn!("Validator {} is already connected, replacing old connection", hotkey);
        // In production, you might want to reject duplicate connections
        // instead of replacing them
    }

    // Check rate limiting
    if let Err(e) = check_connection_rate_limit(hotkey, state).await {
        return Err(anyhow::anyhow!("Rate limit exceeded: {}", e));
    }

    Ok(())
}

/// Check connection rate limits to prevent abuse
async fn check_connection_rate_limit(hotkey: &str, state: &AppState) -> Result<(), anyhow::Error> {
    let connection_count = state.get_validator_connection_attempts(hotkey).await;
    
    // Allow max 10 connections per minute per validator
    if connection_count > 10 {
        return Err(anyhow::anyhow!("Too many connection attempts"));
    }

    // Record this connection attempt
    state.record_validator_connection_attempt(hotkey).await;

    Ok(())
}

/// Initialize WebSocket handler services
pub async fn initialize_websocket_services(state: AppState) {
    info!("Initializing WebSocket handler services");

    // Start health check task
    spawn_health_check_task(state.clone()).await;

    // Initialize connection metrics
    state.initialize_websocket_metrics().await;

    info!("WebSocket handler services initialized");
}

/// Get WebSocket handler statistics
pub async fn get_websocket_stats(state: &AppState) -> serde_json::Value {
    let connections = state.get_all_validator_connections().await;
    let total_connections = connections.len();
    
    let active_connections = connections
        .values()
        .filter(|conn| {
            conn.last_heartbeat.elapsed() < std::time::Duration::from_secs(300)
        })
        .count();

    serde_json::json!({
        "total_connections": total_connections,
        "active_connections": active_connections,
        "inactive_connections": total_connections - active_connections,
        "dev_mode": is_dev_mode(),
        "uptime": chrono::Utc::now().timestamp() - state.get_start_time().await
    })
}

/// Handle WebSocket configuration updates
pub async fn update_websocket_config(
    state: &AppState,
    config: serde_json::Value,
) -> Result<(), anyhow::Error> {
    info!("Updating WebSocket configuration");

    // Update max connections if provided
    if let Some(max_connections) = config.get("max_connections").and_then(|v| v.as_u64()) {
        state.set_max_websocket_connections(max_connections as usize).await;
        info!("Updated max WebSocket connections to: {}", max_connections);
    }

    // Update heartbeat interval if provided
    if let Some(heartbeat_interval) = config.get("heartbeat_interval").and_then(|v| v.as_u64()) {
        state.set_heartbeat_interval(std::time::Duration::from_secs(heartbeat_interval)).await;
        info!("Updated heartbeat interval to: {} seconds", heartbeat_interval);
    }

    // Update frame size limits if provided
    if let Some(max_frame_size) = config.get("max_frame_size").and_then(|v| v.as_u64()) {
        state.set_max_frame_size(max_frame_size as usize).await;
        info!("Updated max frame size to: {} bytes", max_frame_size);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_hotkey() {
        // Valid hotkeys
        assert!(validate_hotkey("validator_123").is_ok());
        assert!(validate_hotkey("test_hotkey").is_ok());
        
        // Invalid hotkeys
        assert!(validate_hotkey("").is_err());
        assert!(validate_hotkey("a".repeat(65).as_str()).is_err());
        assert!(validate_hotkey("invalid-hotkey!").is_err());
    }

    #[test]
    fn test_is_dev_mode() {
        // Test default behavior
        let original_dev_mode = std::env::var("DEV_MODE");
        
        std::env::set_var("DEV_MODE", "true");
        assert!(is_dev_mode());
        
        std::env::set_var("DEV_MODE", "false");
        assert!(!is_dev_mode());
        
        // Restore original value
        match original_dev_mode {
            Ok(value) => std::env::set_var("DEV_MODE", value),
            Err(_) => std::env::remove_var("DEV_MODE"),
        }
    }
}
