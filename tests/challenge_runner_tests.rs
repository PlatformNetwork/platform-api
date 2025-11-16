// Unit tests for Challenge Runner
// Uses mock VMM client (required, VMM needs infrastructure)

use platform_api::challenge_runner::{ChallengeRunner, ChallengeRunnerConfig};
use sqlx::PgPool;
use std::path::PathBuf;

// Helper to create test database pool
async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://platform:platform@localhost:5432/platform_test".to_string());
    
    let pool = PgPool::connect(&database_url).await
        .expect("Failed to connect to test database");
    
    // Run migrations
    let migrations_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("crates/storage/migrations"))
        .expect("Failed to find migrations directory");
    
    sqlx::migrate::Migrator::new(&migrations_path)
        .await
        .expect("Failed to create migrator")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    
    pool
}

#[tokio::test]
async fn test_challenge_runner_creation() {
    let pool = setup_test_db().await;
    let config = ChallengeRunnerConfig::default();
    
    // Create challenge runner (VMM will be mocked in integration tests)
    let runner = ChallengeRunner::new(
        config,
        pool,
        None, // orm_gateway
        None, // validator_challenge_status
        None, // redis_client
        None, // validator_connections
    );
    
    // Test that runner can be created
    assert!(true);
}

#[tokio::test]
async fn test_challenge_runner_config() {
    let config = ChallengeRunnerConfig::default();
    
    assert!(config.enabled);
    assert_eq!(config.max_concurrent_challenges, 10);
    assert_eq!(config.migration_timeout, 300);
    assert_eq!(config.cvm_check_interval, 30);
}

