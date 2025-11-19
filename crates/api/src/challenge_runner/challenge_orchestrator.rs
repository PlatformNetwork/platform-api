//! Challenge orchestration and lifecycle management

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use super::runner_core::ChallengeRunnerCore;
use super::runner::types::{ChallengeInstance, ChallengeMetadata, ChallengeStatus};

/// Orchestrates multiple challenge executions and manages their lifecycle
pub struct ChallengeOrchestrator {
    core: Arc<ChallengeRunnerCore>,
    active_challenges: Arc<RwLock<HashMap<String, ChallengeExecution>>>,
    execution_queue: Arc<RwLock<Vec<QueuedChallenge>>>,
    max_concurrent_challenges: usize,
}

/// Represents an active challenge execution
pub struct ChallengeExecution {
    pub compose_hash: String,
    pub status: ChallengeStatus,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub instance: Option<ChallengeInstance>,
    pub progress: f64, // 0.0 to 1.0
    pub current_step: String,
    pub error_message: Option<String>,
}

/// Represents a queued challenge waiting to be executed
pub struct QueuedChallenge {
    pub compose_hash: String,
    pub challenge_spec: Option<Value>,
    pub env_vars: Option<HashMap<String, String>>,
    pub priority: i32, // Higher numbers = higher priority
    pub queued_at: chrono::DateTime<chrono::Utc>,
    pub max_retries: u32,
    pub retry_count: u32,
}

impl ChallengeOrchestrator {
    /// Create new challenge orchestrator
    pub fn new(core: Arc<ChallengeRunnerCore>, max_concurrent_challenges: usize) -> Self {
        Self {
            core,
            active_challenges: Arc::new(RwLock::new(HashMap::new())),
            execution_queue: Arc::new(RwLock::new(Vec::new())),
            max_concurrent_challenges,
        }
    }

    /// Queue a challenge for execution
    pub async fn queue_challenge(
        &self,
        compose_hash: String,
        challenge_spec: Option<Value>,
        env_vars: Option<HashMap<String, String>>,
        priority: i32,
    ) -> Result<()> {
        info!("Queuing challenge for execution: {} (priority: {})", compose_hash, priority);

        // Check if challenge is already running or queued
        {
            let active = self.active_challenges.read().await;
            if active.contains_key(&compose_hash) {
                warn!("Challenge {} is already running", compose_hash);
                return Err(anyhow::anyhow!("Challenge is already running"));
            }
        }

        {
            let queue = self.execution_queue.read().await;
            if queue.iter().any(|q| q.compose_hash == compose_hash) {
                warn!("Challenge {} is already queued", compose_hash);
                return Err(anyhow::anyhow!("Challenge is already queued"));
            }
        }

        let queued_challenge = QueuedChallenge {
            compose_hash,
            challenge_spec,
            env_vars,
            priority,
            queued_at: chrono::Utc::now(),
            max_retries: 3,
            retry_count: 0,
        };

        {
            let mut queue = self.execution_queue.write().await;
            queue.push(queued_challenge);
            // Sort by priority (highest first)
            queue.sort_by(|a, b| b.priority.cmp(&a.priority));
        }

        // Try to execute queued challenges
        self.process_queue().await?;

        Ok(())
    }

    /// Execute a challenge immediately (bypasses queue)
    pub async fn execute_challenge_immediately(
        &self,
        compose_hash: &str,
        challenge_spec: Option<Value>,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<()> {
        info!("Executing challenge immediately: {}", compose_hash);

        // Check if we have capacity
        let active_count = self.active_challenges.read().await.len();
        if active_count >= self.max_concurrent_challenges {
            return Err(anyhow::anyhow!("Maximum concurrent challenges reached"));
        }

        // Check if challenge is already running
        {
            let active = self.active_challenges.read().await;
            if active.contains_key(compose_hash) {
                return Err(anyhow::anyhow!("Challenge is already running"));
            }
        }

        // Create execution record
        let execution = ChallengeExecution {
            compose_hash: compose_hash.to_string(),
            status: ChallengeStatus::Pending,
            started_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            instance: None,
            progress: 0.0,
            current_step: "Initializing".to_string(),
            error_message: None,
        };

        {
            let mut active = self.active_challenges.write().await;
            active.insert(compose_hash.to_string(), execution);
        }

        // Execute challenge in background
        let core = self.core.clone();
        let active_challenges = self.active_challenges.clone();
        let compose_hash = compose_hash.to_string();

        tokio::spawn(async move {
            let result = execute_challenge_with_tracking(
                core,
                &compose_hash,
                challenge_spec,
                env_vars,
                active_challenges,
            ).await;

            if let Err(e) = result {
                error!("Challenge execution failed: {}", e);
            }
        });

        Ok(())
    }

    /// Stop a running challenge
    pub async fn stop_challenge(&self, compose_hash: &str) -> Result<()> {
        info!("Stopping challenge: {}", compose_hash);

        // Remove from active challenges
        {
            let mut active = self.active_challenges.write().await;
            active.remove(compose_hash);
        }

        // Stop the actual challenge
        self.core.stop_challenge(compose_hash).await?;

        // Process queue to potentially start next challenge
        self.process_queue().await?;

        Ok(())
    }

    /// Get status of all challenges
    pub async fn get_all_challenges_status(&self) -> HashMap<String, ChallengeStatus> {
        let mut status = HashMap::new();

        // Add active challenges
        let active = self.active_challenges.read().await;
        for (compose_hash, execution) in active.iter() {
            status.insert(compose_hash.clone(), execution.status.clone());
        }

        // Add queued challenges
        let queue = self.execution_queue.read().await;
        for queued in queue.iter() {
            status.insert(queued.compose_hash.clone(), ChallengeStatus::Pending);
        }

        status
    }

    /// Get detailed execution info for a challenge
    pub async fn get_challenge_execution(&self, compose_hash: &str) -> Option<ChallengeExecution> {
        let active = self.active_challenges.read().await;
        active.get(compose_hash).cloned()
    }

    /// Get queue information
    pub async fn get_queue_info(&self) -> Vec<QueuedChallenge> {
        let queue = self.execution_queue.read().await;
        queue.clone()
    }

    /// Process the execution queue
    async fn process_queue(&self) -> Result<()> {
        debug!("Processing challenge execution queue");

        let active_count = self.active_challenges.read().await.len();
        if active_count >= self.max_concurrent_challenges {
            debug!("Maximum concurrent challenges reached, skipping queue processing");
            return Ok(());
        }

        let available_slots = self.max_concurrent_challenges - active_count;
        let mut challenges_to_start = Vec::new();

        // Get challenges from queue
        {
            let mut queue = self.execution_queue.write().await;
            for _ in 0..available_slots {
                if let Some(queued) = queue.pop() {
                    challenges_to_start.push(queued);
                } else {
                    break;
                }
            }
        }

        // Start the challenges
        for queued in challenges_to_start {
            if let Err(e) = self.execute_challenge_immediately(
                &queued.compose_hash,
                queued.challenge_spec,
                queued.env_vars,
            ).await {
                error!("Failed to start queued challenge {}: {}", queued.compose_hash, e);
                
                // Re-queue if retries available
                if queued.retry_count < queued.max_retries {
                    let mut retry_queued = queued;
                    retry_queued.retry_count += 1;
                    
                    let mut queue = self.execution_queue.write().await;
                    queue.push(retry_queued);
                    queue.sort_by(|a, b| b.priority.cmp(&a.priority));
                }
            }
        }

        Ok(())
    }

    /// Cleanup completed or failed challenges
    pub async fn cleanup_completed_challenges(&self) -> Result<usize> {
        debug!("Cleaning up completed challenges");

        let mut to_remove = Vec::new();

        {
            let active = self.active_challenges.read().await;
            for (compose_hash, execution) in active.iter() {
                let is_completed = matches!(execution.status, ChallengeStatus::Completed);
                let is_failed = matches!(execution.status, ChallengeStatus::Failed);
                let is_old = execution.updated_at < chrono::Utc::now() - chrono::Duration::hours(1);

                if (is_completed || is_failed) && is_old {
                    to_remove.push(compose_hash.clone());
                }
            }
        }

        // Remove old challenges
        {
            let mut active = self.active_challenges.write().await;
            for compose_hash in &to_remove {
                active.remove(compose_hash);
            }
        }

        if !to_remove.is_empty() {
            info!("Cleaned up {} completed challenges", to_remove.len());
            
            // Process queue to potentially start new challenges
            self.process_queue().await?;
        }

        Ok(to_remove.len())
    }

    /// Get orchestrator statistics
    pub async fn get_statistics(&self) -> OrchestratorStats {
        let active = self.active_challenges.read().await;
        let queue = self.execution_queue.read().await;

        let running_count = active.values()
            .filter(|e| matches!(e.status, ChallengeStatus::Running))
            .count();
        
        let completed_count = active.values()
            .filter(|e| matches!(e.status, ChallengeStatus::Completed))
            .count();
        
        let failed_count = active.values()
            .filter(|e| matches!(e.status, ChallengeStatus::Failed))
            .count();

        OrchestratorStats {
            active_challenges: active.len(),
            running_challenges: running_count,
            completed_challenges: completed_count,
            failed_challenges: failed_count,
            queued_challenges: queue.len(),
            max_concurrent_challenges: self.max_concurrent_challenges,
        }
    }
}

/// Execute a challenge with progress tracking
async fn execute_challenge_with_tracking(
    core: Arc<ChallengeRunnerCore>,
    compose_hash: &str,
    challenge_spec: Option<Value>,
    env_vars: Option<HashMap<String, String>>,
    active_challenges: Arc<RwLock<HashMap<String, ChallengeExecution>>>,
) -> Result<()> {
    // Update status to running
    {
        let mut active = active_challenges.write().await;
        if let Some(execution) = active.get_mut(compose_hash) {
            execution.status = ChallengeStatus::Running;
            execution.current_step = "Executing challenge".to_string();
            execution.updated_at = chrono::Utc::now();
        }
    }

    // Execute the challenge
    let result = core.run_challenge_by_compose_hash_with_spec(
        compose_hash,
        challenge_spec,
        env_vars,
    ).await;

    // Update final status
    {
        let mut active = active_challenges.write().await;
        if let Some(execution) = active.get_mut(compose_hash) {
            match result {
                Ok(_) => {
                    execution.status = ChallengeStatus::Completed;
                    execution.progress = 1.0;
                    execution.current_step = "Completed".to_string();
                }
                Err(e) => {
                    execution.status = ChallengeStatus::Failed;
                    execution.error_message = Some(e.to_string());
                    execution.current_step = "Failed".to_string();
                }
            }
            execution.updated_at = chrono::Utc::now();
        }
    }

    result
}

/// Orchestrator statistics
#[derive(serde::Serialize)]
pub struct OrchestratorStats {
    pub active_challenges: usize,
    pub running_challenges: usize,
    pub completed_challenges: usize,
    pub failed_challenges: usize,
    pub queued_challenges: usize,
    pub max_concurrent_challenges: usize,
}
