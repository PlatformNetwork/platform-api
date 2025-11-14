// Integration tests for job distribution
use platform_api_models::*;
use platform_api_api::job_distributor::{JobDistributor, DistributeJobRequest};
use platform_api_api::state::AppState;
use platform_api_scheduler::{SchedulerService, CreateJobRequest};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Mock WebSocket message handler for testing
struct MockWebSocketHandler {
    received_messages: Arc<RwLock<Vec<serde_json::Value>>>,
}

impl MockWebSocketHandler {
    fn new() -> Self {
        Self {
            received_messages: Arc::new(RwLock::new(Vec::new())),
        }
    }
    
    async fn get_messages(&self) -> Vec<serde_json::Value> {
        self.received_messages.read().await.clone()
    }
    
    async fn handle_message(&self, msg: serde_json::Value) {
        self.received_messages.write().await.push(msg);
    }
}

#[tokio::test]
async fn test_job_distribution_to_single_validator() {
    // Setup test state
    let state = create_test_state().await;
    let distributor = JobDistributor::new(state.clone());
    
    // Add a validator
    let validator_hotkey = "test_validator_1";
    let compose_hash = "test_compose_hash";
    state.add_validator(compose_hash, validator_hotkey).await;
    
    // Create distribution request
    let job_id = Uuid::new_v4().to_string();
    let request = DistributeJobRequest {
        job_id: job_id.clone(),
        job_name: "test_job".to_string(),
        payload: json!({
            "task": "evaluate",
            "params": {"model": "test_model"}
        }),
        compose_hash: compose_hash.to_string(),
        challenge_id: "test_challenge".to_string(),
        challenge_cvm_ws_url: Some("ws://challenge:8080".to_string()),
    };
    
    // Distribute job
    let result = distributor.distribute_job_to_validators(request).await
        .expect("Failed to distribute job");
    
    // Verify distribution
    assert!(result.distributed);
    assert_eq!(result.validator_count, 1);
    assert_eq!(result.assigned_validators.len(), 1);
    assert_eq!(result.assigned_validators[0], validator_hotkey);
    assert_eq!(result.job_id, job_id);
    
    // Verify job is in cache
    let job_cache = state.job_cache.read().await;
    assert!(job_cache.contains_key(&job_id));
    
    let cached_job = &job_cache[&job_id];
    assert_eq!(cached_job.status, JobStatus::Running);
    assert_eq!(cached_job.assigned_validators.len(), 1);
}

#[tokio::test]
async fn test_job_distribution_to_multiple_validators() {
    let state = create_test_state().await;
    let distributor = JobDistributor::new(state.clone());
    
    // Add multiple validators
    let compose_hash = "multi_validator_hash";
    let validator_count = 5;
    let mut expected_validators = Vec::new();
    
    for i in 0..validator_count {
        let hotkey = format!("validator_{}", i);
        expected_validators.push(hotkey.clone());
        state.add_validator(compose_hash, &hotkey).await;
    }
    
    // Create and distribute job
    let job_id = Uuid::new_v4().to_string();
    let request = DistributeJobRequest {
        job_id: job_id.clone(),
        job_name: "parallel_job".to_string(),
        payload: json!({
            "task": "distributed_training",
            "shards": validator_count
        }),
        compose_hash: compose_hash.to_string(),
        challenge_id: "distributed_challenge".to_string(),
        challenge_cvm_ws_url: None,
    };
    
    let result = distributor.distribute_job_to_validators(request).await
        .expect("Failed to distribute job");
    
    // Verify all validators received the job
    assert!(result.distributed);
    assert_eq!(result.validator_count, validator_count);
    assert_eq!(result.assigned_validators.len(), validator_count);
    
    // Check all expected validators are assigned
    for validator in &expected_validators {
        assert!(result.assigned_validators.contains(validator));
    }
}

#[tokio::test]
async fn test_job_distribution_with_no_validators() {
    let state = create_test_state().await;
    let distributor = JobDistributor::new(state.clone());
    
    // Don't add any validators
    let compose_hash = "empty_validator_hash";
    
    let request = DistributeJobRequest {
        job_id: Uuid::new_v4().to_string(),
        job_name: "lonely_job".to_string(),
        payload: json!({"task": "impossible"}),
        compose_hash: compose_hash.to_string(),
        challenge_id: "no_validators_challenge".to_string(),
        challenge_cvm_ws_url: None,
    };
    
    let result = distributor.distribute_job_to_validators(request).await
        .expect("Failed to distribute job");
    
    // Should handle gracefully with no distribution
    assert!(!result.distributed);
    assert_eq!(result.validator_count, 0);
    assert_eq!(result.assigned_validators.len(), 0);
}

#[tokio::test]
async fn test_job_result_forwarding() {
    let state = create_test_state().await;
    let distributor = JobDistributor::new(state.clone());
    
    // Setup job with challenge websocket URL
    let job_id = Uuid::new_v4().to_string();
    let validator_hotkey = "result_validator";
    let compose_hash = "result_compose_hash";
    
    state.add_validator(compose_hash, validator_hotkey).await;
    
    // Distribute job
    let request = DistributeJobRequest {
        job_id: job_id.clone(),
        job_name: "result_test_job".to_string(),
        payload: json!({"task": "compute"}),
        compose_hash: compose_hash.to_string(),
        challenge_id: "result_challenge".to_string(),
        challenge_cvm_ws_url: Some("ws://challenge:8080".to_string()),
    };
    
    let dist_result = distributor.distribute_job_to_validators(request).await
        .expect("Failed to distribute job");
    
    assert!(dist_result.distributed);
    
    // Simulate job result
    let job_result = platform_api_api::job_distributor::JobResult {
        job_id: job_id.clone(),
        result: json!({
            "score": 0.95,
            "metrics": {"accuracy": 0.95, "speed": 1.5}
        }),
        error: None,
        validator_hotkey: Some("test_validator".to_string()),
    };
    
    // Forward result
    distributor.forward_job_result(job_result).await
        .expect("Failed to forward job result");
    
    // Verify job cache shows completion
    let job_cache = state.job_cache.read().await;
    let cached_job = &job_cache[&job_id];
    assert_eq!(cached_job.status, JobStatus::Completed);
}

#[tokio::test]
async fn test_validator_disconnection_handling() {
    let state = create_test_state().await;
    let distributor = JobDistributor::new(state.clone());
    
    // Add validators
    let compose_hash = "disconnect_test_hash";
    let validator1 = "validator_stable";
    let validator2 = "validator_disconnect";
    
    state.add_validator(compose_hash, validator1).await;
    state.add_validator(compose_hash, validator2).await;
    
    // Distribute job
    let job_id = Uuid::new_v4().to_string();
    let request = DistributeJobRequest {
        job_id: job_id.clone(),
        job_name: "disconnect_test".to_string(),
        payload: json!({"task": "long_running"}),
        compose_hash: compose_hash.to_string(),
        challenge_id: "disconnect_challenge".to_string(),
        challenge_cvm_ws_url: None,
    };
    
    let result = distributor.distribute_job_to_validators(request).await
        .expect("Failed to distribute job");
    
    assert_eq!(result.validator_count, 2);
    
    // Simulate validator disconnection
    state.remove_validator(compose_hash, validator2).await;
    
    // Verify remaining validator count
    assert_eq!(state.get_validator_count(compose_hash).await, 1);
    
    // Job should still be tracked
    let job_cache = state.job_cache.read().await;
    assert!(job_cache.contains_key(&job_id));
}

#[tokio::test]
async fn test_concurrent_job_distribution() {
    use futures::future::join_all;
    
    let state = create_test_state().await;
    let distributor = Arc::new(JobDistributor::new(state.clone()));
    
    // Add validators
    let compose_hash = "concurrent_test_hash";
    for i in 0..3 {
        state.add_validator(compose_hash, &format!("concurrent_validator_{}", i)).await;
    }
    
    // Create multiple jobs
    let job_count = 10;
    let mut distribution_tasks = Vec::new();
    
    for i in 0..job_count {
        let distributor = distributor.clone();
        let job_id = format!("concurrent_job_{}", i);
        
        let request = DistributeJobRequest {
            job_id,
            job_name: format!("concurrent_task_{}", i),
            payload: json!({"task_num": i}),
            compose_hash: compose_hash.to_string(),
            challenge_id: "concurrent_challenge".to_string(),
            challenge_cvm_ws_url: None,
        };
        
        distribution_tasks.push(tokio::spawn(async move {
            distributor.distribute_job_to_validators(request).await
        }));
    }
    
    // Wait for all distributions
    let results = join_all(distribution_tasks).await;
    
    // Verify all succeeded
    for (i, result) in results.iter().enumerate() {
        let dist_result = result.as_ref().unwrap().as_ref().unwrap();
        assert!(dist_result.distributed, "Job {} failed to distribute", i);
        assert_eq!(dist_result.validator_count, 3);
    }
    
    // Verify all jobs are in cache
    let job_cache = state.job_cache.read().await;
    assert_eq!(job_cache.len(), job_count);
}

#[tokio::test]
async fn test_job_priority_distribution() {
    let state = create_test_state().await;
    
    // Create scheduler with database
    let pool = create_test_db_pool().await;
    let scheduler = Arc::new(
        platform_api_scheduler::SchedulerService::with_database(
            &platform_api_scheduler::SchedulerConfig::default(),
            Arc::new(pool.clone())
        ).expect("Failed to create scheduler")
    );
    
    // Replace state scheduler
    let state = AppState::builder()
        .from_existing(state)
        .scheduler(scheduler)
        .build();
    
    let distributor = JobDistributor::new(state.clone());
    
    // Add validator
    let compose_hash = "priority_test_hash";
    let validator = "priority_validator";
    state.add_validator(compose_hash, validator).await;
    
    // Create challenge
    let challenge_id = Uuid::new_v4();
    
    // Create jobs with different priorities
    let priorities = vec![
        JobPriority::Low,
        JobPriority::Critical,
        JobPriority::Normal,
        JobPriority::High,
    ];
    
    let mut job_ids = Vec::new();
    
    for (i, priority) in priorities.iter().enumerate() {
        let job = state.scheduler.create_job(CreateJobRequest {
            challenge_id: Id::from(challenge_id),
            payload: json!({"job_num": i, "priority": format!("{:?}", priority)}),
            priority: Some(priority.clone()),
            runtime: RuntimeType::Docker,
            timeout: Some(300),
            max_retries: Some(1),
        }).await.expect("Failed to create job");
        
        job_ids.push(job.id.to_string());
    }
    
    // Distribute jobs - higher priority should be distributed first
    for job_id in &job_ids {
        let request = DistributeJobRequest {
            job_id: job_id.clone(),
            job_name: "priority_job".to_string(),
            payload: json!({"job_id": job_id}),
            compose_hash: compose_hash.to_string(),
            challenge_id: challenge_id.to_string(),
            challenge_cvm_ws_url: None,
        };
        
        distributor.distribute_job_to_validators(request).await
            .expect("Failed to distribute priority job");
    }
    
    // Cleanup
    pool.close().await;
}

#[tokio::test]
async fn test_job_retry_distribution() {
    let state = create_test_state().await;
    let distributor = JobDistributor::new(state.clone());
    
    // Add validator
    let compose_hash = "retry_test_hash";
    let validator = "retry_validator";
    state.add_validator(compose_hash, validator).await;
    
    // First distribution
    let job_id = Uuid::new_v4().to_string();
    let request = DistributeJobRequest {
        job_id: job_id.clone(),
        job_name: "retry_job".to_string(),
        payload: json!({"attempt": 1}),
        compose_hash: compose_hash.to_string(),
        challenge_id: "retry_challenge".to_string(),
        challenge_cvm_ws_url: None,
    };
    
    let result1 = distributor.distribute_job_to_validators(request.clone()).await
        .expect("Failed to distribute job");
    
    assert!(result1.distributed);
    
    // Simulate job failure
    let job_result = platform_api_api::job_distributor::JobResult {
        job_id: job_id.clone(),
        result: json!({}),
        error: Some("Job failed".to_string()),
        validator_hotkey: Some("test_validator".to_string()),
    };
    
    distributor.forward_job_result(job_result).await
        .expect("Failed to forward failure result");
    
    // Retry distribution with updated payload
    let retry_request = DistributeJobRequest {
        payload: json!({"attempt": 2}),
        ..request
    };
    
    let result2 = distributor.distribute_job_to_validators(retry_request).await
        .expect("Failed to distribute retry");
    
    assert!(result2.distributed);
    assert_eq!(result2.job_id, job_id);
}

// Helper functions

async fn create_test_state() -> AppState {
    use platform_api_storage::{StorageBackend, MemoryStorageBackend};
    use platform_api_attestation::TdxConfig;
    
    // Use in-memory storage for tests
    let storage = Arc::new(MemoryStorageBackend::new());
    
    let scheduler = Arc::new(
        platform_api_scheduler::SchedulerService::new(
            &platform_api_scheduler::SchedulerConfig::default()
        ).expect("Failed to create scheduler")
    );
    
    let attestation = Arc::new(
        platform_api_attestation::AttestationService::new(
            &TdxConfig::from_env()
        ).expect("Failed to create attestation")
    );
    
    AppState::builder()
        .storage(storage)
        .scheduler(scheduler)
        .attestation(attestation)
        .build()
}

async fn create_test_db_pool() -> sqlx::PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://platform:platform@localhost:5432/platform_test".to_string());
    
    let pool = sqlx::PgPool::connect(&database_url).await
        .expect("Failed to connect to test database");
    
    // Run migrations
    sqlx::migrate!("./crates/storage/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    
    pool
}
