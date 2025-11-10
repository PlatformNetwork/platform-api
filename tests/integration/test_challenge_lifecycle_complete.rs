// Complete integration tests for challenge lifecycle
use platform_api_models::*;
use platform_api_scheduler::CreateJobRequest;
use platform_api_storage::{StorageBackend, PostgresStorageBackend};
use platform_api_api::state::AppState;
use platform_api_api::job_distributor::{JobDistributor, DistributeJobRequest};
use sqlx::PgPool;
use uuid::Uuid;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::time::{sleep, Duration};
use serde_json::json;

/// Test helper to create app state
async fn create_test_state(pool: PgPool) -> AppState {
    let storage = Arc::new(PostgresStorageBackend::new(
        &std::env::var("DATABASE_URL").unwrap_or_else(|_| 
            "postgresql://platform:platform@localhost:5432/platform_test".to_string()
        )
    ).await.expect("Failed to create storage"));
    
    let scheduler = Arc::new(platform_api_scheduler::SchedulerService::with_database(
        &platform_api_scheduler::SchedulerConfig::default(),
        Arc::new(pool.clone())
    ).expect("Failed to create scheduler"));
    
    let attestation = Arc::new(platform_api_attestation::AttestationService::new(
        &platform_api_attestation::TdxConfig::from_env()
    ).expect("Failed to create attestation service"));
    
    AppState::builder()
        .storage(storage)
        .scheduler(scheduler)
        .attestation(attestation)
        .database_pool(Some(Arc::new(pool)))
        .build()
}

#[tokio::test]
async fn test_complete_challenge_lifecycle() {
    // Setup database
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://platform:platform@localhost:5432/platform_test".to_string());
    
    let pool = PgPool::connect(&database_url).await
        .expect("Failed to connect to test database");
    
    // Run migrations
    sqlx::migrate!("./crates/storage/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    
    // Create app state
    let state = create_test_state(pool.clone()).await;
    
    // 1. Register a challenge
    let challenge = CreateChallengeRequest {
        name: "test-challenge".to_string(),
        description: Some("Test challenge for integration testing".to_string()),
        compose_yaml: r#"
version: '3.8'
services:
  challenge:
    image: test-challenge:latest
    environment:
      - CHALLENGE_MODE=production
"#.to_string(),
        github_repo: Some("https://github.com/test/challenge".to_string()),
        mechanism_id: 1,
    };
    
    let created_challenge = state.storage.create_challenge(challenge).await
        .expect("Failed to create challenge");
    
    assert!(!created_challenge.id.is_nil());
    assert_eq!(created_challenge.name, "test-challenge");
    
    // 2. Simulate validator connection
    let validator_hotkey = "test_validator_hotkey";
    let compose_hash = created_challenge.compose_hash.clone();
    
    // Add validator to active set
    state.add_validator(&compose_hash, validator_hotkey).await;
    
    // Verify validator is active
    assert_eq!(state.get_validator_count(&compose_hash).await, 1);
    
    // 3. Create and distribute a job
    let job_request = CreateJobRequest {
        challenge_id: Id::from(created_challenge.id),
        payload: json!({
            "agent_hash": "test_agent_123",
            "miner_hotkey": "test_miner_456",
            "job_name": "evaluate_agent"
        }),
        priority: Some(JobPriority::Normal),
        runtime: RuntimeType::Docker,
        timeout: Some(300),
        max_retries: Some(3),
    };
    
    let job = state.scheduler.create_job(job_request.clone()).await
        .expect("Failed to create job");
    
    assert_eq!(job.status, JobStatus::Pending);
    assert_eq!(job.challenge_id, Id::from(created_challenge.id));
    
    // 4. Test job distribution
    let distributor = JobDistributor::new(state.clone());
    
    let distribute_request = DistributeJobRequest {
        job_id: job.id.to_string(),
        job_name: "evaluate_agent".to_string(),
        payload: job_request.payload.clone(),
        compose_hash: compose_hash.clone(),
        challenge_id: created_challenge.id.to_string(),
        challenge_cvm_ws_url: Some("ws://challenge:8080".to_string()),
    };
    
    let distribution_result = distributor.distribute_job_to_validators(distribute_request).await
        .expect("Failed to distribute job");
    
    assert!(distribution_result.distributed);
    assert_eq!(distribution_result.validator_count, 1);
    assert_eq!(distribution_result.assigned_validators.len(), 1);
    assert!(distribution_result.assigned_validators.contains(&validator_hotkey.to_string()));
    
    // 5. Simulate validator claiming the job
    let claimed_job = state.scheduler.claim_job(
        ClaimJobRequest {
            validator_hotkey: Hotkey::from(validator_hotkey),
            challenge_id: Some(Id::from(created_challenge.id)),
            runtime: RuntimeType::Docker,
        }
    ).await.expect("Failed to claim job");
    
    assert_eq!(claimed_job.job.id, job.id);
    assert_eq!(claimed_job.job.status, JobStatus::Claimed);
    assert_eq!(claimed_job.job.validator_hotkey, Some(Hotkey::from(validator_hotkey)));
    
    // 6. Simulate job execution and result submission
    let eval_result = EvalResult {
        score: 0.85,
        metrics: HashMap::from([
            ("accuracy".to_string(), 0.85),
            ("speed".to_string(), 1.23),
        ]),
        passed_tests: 17,
        total_tests: 20,
        execution_time: 45.6,
        validator_hotkey: validator_hotkey.to_string(),
        challenge_id: created_challenge.id.to_string(),
    };
    
    let submit_result = state.scheduler.submit_result(
        job.id.into(),
        SubmitResultRequest {
            result: eval_result.clone(),
            status: JobStatus::Completed,
            error: None,
            usage: Some(ResourceUsage {
                cpu_seconds: 180.0,
                memory_mb_seconds: 2048.0,
                network_bytes: 1024 * 1024,
            }),
        }
    ).await.expect("Failed to submit result");
    
    assert_eq!(submit_result.status, JobStatus::Completed);
    assert!(submit_result.completed_at.is_some());
    
    // 7. Verify job completion and results
    let completed_job = state.scheduler.get_job(job.id.into()).await
        .expect("Failed to get completed job");
    
    assert_eq!(completed_job.status, JobStatus::Completed);
    
    // 8. Test job history and statistics
    let job_stats = state.scheduler.get_job_stats(Some(created_challenge.id.into())).await
        .expect("Failed to get job stats");
    
    assert_eq!(job_stats.total_jobs, 1);
    assert_eq!(job_stats.completed_jobs, 1);
    assert_eq!(job_stats.failed_jobs, 0);
    assert_eq!(job_stats.pending_jobs, 0);
    
    // Cleanup
    state.storage.delete_challenge(created_challenge.id).await
        .expect("Failed to delete challenge");
    
    // Close database connection
    pool.close().await;
}

#[tokio::test]
async fn test_job_failure_and_retry() {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://platform:platform@localhost:5432/platform_test".to_string());
    
    let pool = PgPool::connect(&database_url).await
        .expect("Failed to connect to test database");
    
    // Run migrations
    sqlx::migrate!("./crates/storage/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    
    let state = create_test_state(pool.clone()).await;
    
    // Create challenge
    let challenge = state.storage.create_challenge(CreateChallengeRequest {
        name: "retry-test-challenge".to_string(),
        description: Some("Test challenge for retry logic".to_string()),
        compose_yaml: "version: '3.8'\nservices:\n  test:\n    image: test:latest".to_string(),
        github_repo: None,
        mechanism_id: 1,
    }).await.expect("Failed to create challenge");
    
    // Create job with retries
    let job = state.scheduler.create_job(CreateJobRequest {
        challenge_id: Id::from(challenge.id),
        payload: json!({"test": "retry"}),
        priority: Some(JobPriority::Normal),
        runtime: RuntimeType::Docker,
        timeout: Some(60),
        max_retries: Some(3),
    }).await.expect("Failed to create job");
    
    // Claim job
    let validator_hotkey = "retry_validator";
    state.add_validator(&challenge.compose_hash, validator_hotkey).await;
    
    let claimed = state.scheduler.claim_job(ClaimJobRequest {
        validator_hotkey: Hotkey::from(validator_hotkey),
        challenge_id: Some(Id::from(challenge.id)),
        runtime: RuntimeType::Docker,
    }).await.expect("Failed to claim job");
    
    // Fail the job
    let failed = state.scheduler.fail_job(
        job.id.into(),
        FailJobRequest {
            error: "Simulated failure".to_string(),
            retry: true,
        }
    ).await.expect("Failed to fail job");
    
    assert_eq!(failed.retry_count, 1);
    assert!(failed.retry_count <= failed.max_retries);
    
    // Job should be retryable
    assert_eq!(failed.status, JobStatus::Pending); // Back to pending for retry
    
    // Claim again
    let reclaimed = state.scheduler.claim_job(ClaimJobRequest {
        validator_hotkey: Hotkey::from(validator_hotkey),
        challenge_id: Some(Id::from(challenge.id)),
        runtime: RuntimeType::Docker,
    }).await.expect("Failed to reclaim job");
    
    assert_eq!(reclaimed.job.id, job.id);
    assert_eq!(reclaimed.job.retry_count, 1);
    
    // Cleanup
    state.storage.delete_challenge(challenge.id).await.ok();
    pool.close().await;
}

#[tokio::test]
async fn test_attestation_integration() {
    // Test with dev mode enabled
    std::env::set_var("TEE_ENFORCED", "false");
    std::env::set_var("DEV_MODE", "true");
    
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://platform:platform@localhost:5432/platform_test".to_string());
    
    let pool = PgPool::connect(&database_url).await
        .expect("Failed to connect to test database");
    
    let state = create_test_state(pool.clone()).await;
    
    // Create attestation request
    let attest_request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(vec![0u8; 1024]), // Mock quote
        report: None,
        nonce: vec![0u8; 32],
        measurements: vec![
            Measurement {
                name: "kernel".to_string(),
                value: "test_kernel_hash".to_string(),
                algorithm: "sha256".to_string(),
            }
        ],
        capabilities: vec!["tee".to_string()],
    };
    
    // Verify attestation
    let response = state.attestation.verify_attestation(attest_request).await
        .expect("Failed to verify attestation");
    
    assert_eq!(response.status, AttestationStatus::Verified);
    assert!(!response.session_token.is_empty());
    assert!(response.expires_at > chrono::Utc::now());
    
    // Cleanup env vars
    std::env::remove_var("TEE_ENFORCED");
    std::env::remove_var("DEV_MODE");
    pool.close().await;
}

#[tokio::test]
async fn test_pool_and_node_management() {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://platform:platform@localhost:5432/platform_test".to_string());
    
    let pool = PgPool::connect(&database_url).await
        .expect("Failed to connect to test database");
    
    // Run migrations
    sqlx::migrate!("./crates/storage/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    
    let state = create_test_state(pool.clone()).await;
    
    // Create a pool
    let validator_hotkey = "pool_test_validator";
    let create_pool_req = CreatePoolRequest {
        name: "test-pool".to_string(),
        description: Some("Test pool for integration testing".to_string()),
        autoscale_policy: Some(AutoscalePolicy {
            min_nodes: 1,
            max_nodes: 10,
            scale_up_threshold: 0.8,
            scale_down_threshold: 0.2,
            stabilization_window_minutes: 5,
        }),
        region: Some("us-east-1".to_string()),
    };
    
    let created_pool = state.storage.create_pool(validator_hotkey, &create_pool_req).await
        .expect("Failed to create pool");
    
    assert_eq!(created_pool.name, "test-pool");
    assert_eq!(created_pool.validator_hotkey, validator_hotkey);
    
    // Add nodes to the pool
    let node1 = state.storage.add_node(created_pool.id, &AddNodeRequest {
        name: "node-1".to_string(),
        vmm_url: "http://vmm1:8080".to_string(),
        capacity: NodeCapacity {
            total_cpu: 8,
            total_memory_gb: 16,
            total_disk_gb: 100,
            available_cpu: 8,
            available_memory_gb: 16,
            available_disk_gb: 100,
            has_tdx: true,
            has_gpu: false,
            gpu_count: 0,
            gpu_model: None,
            region: "us-east-1".to_string(),
        },
        metadata: Some(json!({"provider": "aws", "instance_type": "m5.xlarge"})),
    }).await.expect("Failed to add node");
    
    assert_eq!(node1.pool_id, created_pool.id);
    assert_eq!(node1.vmm_url, "http://vmm1:8080");
    
    // Get pool capacity
    let capacity = state.storage.get_pool_capacity(created_pool.id).await
        .expect("Failed to get pool capacity");
    
    assert_eq!(capacity.total_nodes, 1);
    assert_eq!(capacity.total_cpu, 8);
    assert!(capacity.has_tdx);
    
    // List pools
    let pools = state.storage.list_pools(Some(validator_hotkey), 1, 10).await
        .expect("Failed to list pools");
    
    assert_eq!(pools.total, 1);
    assert_eq!(pools.pools[0].id, created_pool.id);
    
    // Update pool
    let updated = state.storage.update_pool(created_pool.id, &UpdatePoolRequest {
        name: Some("updated-pool".to_string()),
        description: None,
        autoscale_policy: None,
        region: None,
    }).await.expect("Failed to update pool");
    
    assert_eq!(updated.name, "updated-pool");
    
    // Delete node
    state.storage.delete_node(node1.id).await
        .expect("Failed to delete node");
    
    // Delete pool
    state.storage.delete_pool(created_pool.id).await
        .expect("Failed to delete pool");
    
    pool.close().await;
}

#[tokio::test]
async fn test_websocket_job_distribution() {
    use tokio::sync::RwLock;
    use std::collections::HashSet;
    
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://platform:platform@localhost:5432/platform_test".to_string());
    
    let pool = PgPool::connect(&database_url).await
        .expect("Failed to connect to test database");
    
    let state = create_test_state(pool.clone()).await;
    
    // Create challenge
    let challenge = state.storage.create_challenge(CreateChallengeRequest {
        name: "ws-test-challenge".to_string(),
        description: Some("WebSocket test challenge".to_string()),
        compose_yaml: "version: '3.8'\nservices:\n  test:\n    image: test:latest".to_string(),
        github_repo: None,
        mechanism_id: 1,
    }).await.expect("Failed to create challenge");
    
    // Simulate multiple validators
    let validator_count = 3;
    let mut validators = Vec::new();
    
    for i in 0..validator_count {
        let hotkey = format!("ws_validator_{}", i);
        validators.push(hotkey.clone());
        state.add_validator(&challenge.compose_hash, &hotkey).await;
    }
    
    // Create job
    let job = state.scheduler.create_job(CreateJobRequest {
        challenge_id: Id::from(challenge.id),
        payload: json!({
            "task": "distributed_evaluation",
            "params": {"complexity": "high"}
        }),
        priority: Some(JobPriority::High),
        runtime: RuntimeType::Docker,
        timeout: Some(600),
        max_retries: Some(1),
    }).await.expect("Failed to create job");
    
    // Distribute job
    let distributor = JobDistributor::new(state.clone());
    
    let result = distributor.distribute_job_to_validators(DistributeJobRequest {
        job_id: job.id.to_string(),
        job_name: "distributed_evaluation".to_string(),
        payload: json!({"task": "distributed_evaluation"}),
        compose_hash: challenge.compose_hash.clone(),
        challenge_id: challenge.id.to_string(),
        challenge_cvm_ws_url: None,
    }).await.expect("Failed to distribute job");
    
    assert!(result.distributed);
    assert_eq!(result.validator_count, validator_count);
    assert_eq!(result.assigned_validators.len(), validator_count);
    
    // Verify job is in cache
    {
        let cache = state.job_cache.read().await;
        assert!(cache.contains_key(&job.id.to_string()));
    }
    
    // Cleanup
    state.storage.delete_challenge(challenge.id).await.ok();
    pool.close().await;
}
