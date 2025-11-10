// Integration test for challenge environment variables encryption system
// Tests that env vars can be stored encrypted and loaded decrypted

use platform_api::state::{AppConfig, AppState};
use platform_api_storage::StorageConfig;
use std::collections::HashMap;

#[tokio::test]
#[ignore] // Requires database setup
async fn test_challenge_env_vars_storage_and_retrieval() {
    // This test requires a database connection
    // It should be run with proper database setup
    
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for this test");
    
    // Create a test encryption key (32 bytes hex encoded = 64 chars)
    let encryption_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    
    let storage_config = StorageConfig {
        backend_type: "postgres".to_string(),
        encryption_key: encryption_key.to_string(),
    };
    
    let config = AppConfig {
        server_port: 8000,
        server_host: "localhost".to_string(),
        jwt_secret_ui: "test-secret".to_string(),
        database_url: database_url.clone(),
        storage_config: storage_config.clone(),
        attestation_config: platform_api_attestation::AttestationConfig::default(),
        kbs_config: platform_api_kbs::KbsConfig::default(),
        scheduler_config: platform_api_scheduler::SchedulerConfig::default(),
        builder_config: platform_api_builder::BuilderConfig::default(),
        metrics_config: platform_api::state::MetricsConfig {
            enabled: false,
            port: 9090,
            path: "/metrics".to_string(),
            collect_interval: 60,
        },
    };
    
    let state = AppState::new(config).await.expect("Failed to create AppState");
    
    // Test compose_hash
    let compose_hash = "test-compose-hash-12345";
    
    // Store some test environment variables
    state
        .store_challenge_env_var(compose_hash, "TEST_KEY_1", "test_value_1")
        .await
        .expect("Failed to store env var 1");
    
    state
        .store_challenge_env_var(compose_hash, "TEST_KEY_2", "test_value_2")
        .await
        .expect("Failed to store env var 2");
    
    state
        .store_challenge_env_var(compose_hash, "SENSITIVE_TOKEN", "secret_token_123")
        .await
        .expect("Failed to store sensitive env var");
    
    // Load the environment variables
    let loaded_vars = state
        .load_challenge_env_vars(compose_hash)
        .await
        .expect("Failed to load env vars");
    
    // Verify all variables were loaded correctly
    assert_eq!(loaded_vars.len(), 3, "Should have loaded 3 environment variables");
    assert_eq!(
        loaded_vars.get("TEST_KEY_1"),
        Some(&"test_value_1".to_string()),
        "TEST_KEY_1 should match"
    );
    assert_eq!(
        loaded_vars.get("TEST_KEY_2"),
        Some(&"test_value_2".to_string()),
        "TEST_KEY_2 should match"
    );
    assert_eq!(
        loaded_vars.get("SENSITIVE_TOKEN"),
        Some(&"secret_token_123".to_string()),
        "SENSITIVE_TOKEN should match"
    );
    
    // Test updating an existing variable
    state
        .store_challenge_env_var(compose_hash, "TEST_KEY_1", "updated_value_1")
        .await
        .expect("Failed to update env var");
    
    let updated_vars = state
        .load_challenge_env_vars(compose_hash)
        .await
        .expect("Failed to load updated env vars");
    
    assert_eq!(
        updated_vars.get("TEST_KEY_1"),
        Some(&"updated_value_1".to_string()),
        "TEST_KEY_1 should be updated"
    );
    assert_eq!(updated_vars.len(), 3, "Should still have 3 variables");
    
    println!("✅ All environment variable storage and retrieval tests passed!");
}

#[tokio::test]
#[ignore] // Requires database setup
async fn test_challenge_env_vars_empty_challenge() {
    // Test loading env vars for a challenge that has none
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for this test");
    
    let encryption_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    
    let storage_config = StorageConfig {
        backend_type: "postgres".to_string(),
        encryption_key: encryption_key.to_string(),
    };
    
    let config = AppConfig {
        server_port: 8000,
        server_host: "localhost".to_string(),
        jwt_secret_ui: "test-secret".to_string(),
        database_url: database_url.clone(),
        storage_config: storage_config.clone(),
        attestation_config: platform_api_attestation::AttestationConfig::default(),
        kbs_config: platform_api_kbs::KbsConfig::default(),
        scheduler_config: platform_api_scheduler::SchedulerConfig::default(),
        builder_config: platform_api_builder::BuilderConfig::default(),
        metrics_config: platform_api::state::MetricsConfig {
            enabled: false,
            port: 9090,
            path: "/metrics".to_string(),
            collect_interval: 60,
        },
    };
    
    let state = AppState::new(config).await.expect("Failed to create AppState");
    
    // Try to load env vars for a challenge that doesn't have any
    let empty_hash = "non-existent-compose-hash";
    let loaded_vars = state
        .load_challenge_env_vars(empty_hash)
        .await
        .expect("Should succeed even if no vars exist");
    
    assert_eq!(
        loaded_vars.len(),
        0,
        "Should return empty HashMap for challenge with no env vars"
    );
    
    println!("✅ Empty challenge env vars test passed!");
}

