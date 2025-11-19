//! CVM migration handling

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Migration from CVM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Migration {
    pub version: String,
    pub name: String,
    pub sql: String,
    pub checksum: String,
}

/// Get migrations from CVM
pub async fn get_migrations_from_cvm(cvm_api_url: &str) -> Result<Vec<Migration>> {
    debug!(cvm_api_url = cvm_api_url, "Getting migrations from CVM");

    let migrations_url = format!("{}/migrations", cvm_api_url);

    let client = reqwest::Client::new();
    let response = client
        .get(&migrations_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get migrations from CVM")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get migrations: {}", error_text));
    }

    let migrations: Vec<Migration> = response.json().await
        .context("Failed to parse migrations")?;

    info!(
        cvm_api_url = cvm_api_url,
        migration_count = migrations.len(),
        "Retrieved migrations from CVM"
    );

    Ok(migrations)
}

/// Apply migration to CVM
pub async fn apply_migration_to_cvm(
    cvm_api_url: &str,
    migration: &Migration,
) -> Result<()> {
    info!(
        cvm_api_url = cvm_api_url,
        version = migration.version,
        "Applying migration to CVM"
    );

    let apply_url = format!("{}/migrations/apply", cvm_api_url);

    let client = reqwest::Client::new();
    let response = client
        .post(&apply_url)
        .header("Content-Type", "application/json")
        .json(migration)
        .send()
        .await
        .context("Failed to apply migration to CVM")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to apply migration: {}", error_text));
    }

    info!(
        cvm_api_url = cvm_api_url,
        version = migration.version,
        "Migration applied to CVM successfully"
    );

    Ok(())
}

/// Get migration status from CVM
pub async fn get_cvm_migration_status(cvm_api_url: &str) -> Result<CvmMigrationStatus> {
    debug!(cvm_api_url = cvm_api_url, "Getting CVM migration status");

    let status_url = format!("{}/migrations/status", cvm_api_url);

    let client = reqwest::Client::new();
    let response = client
        .get(&status_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get migration status")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get migration status: {}", error_text));
    }

    let status: CvmMigrationStatus = response.json().await
        .context("Failed to parse migration status")?;

    debug!(
        cvm_api_url = cvm_api_url,
        status = %status.status,
        applied_count = status.applied_migrations.len(),
        "Retrieved CVM migration status"
    );

    Ok(status)
}

/// Validate migration with CVM
pub async fn validate_migration_with_cvm(
    cvm_api_url: &str,
    migration: &Migration,
) -> Result<MigrationValidationResult> {
    debug!(
        cvm_api_url = cvm_api_url,
        version = migration.version,
        "Validating migration with CVM"
    );

    let validate_url = format!("{}/migrations/validate", cvm_api_url);

    let client = reqwest::Client::new();
    let response = client
        .post(&validate_url)
        .header("Content-Type", "application/json")
        .json(migration)
        .send()
        .await
        .context("Failed to validate migration")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to validate migration: {}", error_text));
    }

    let validation_result: MigrationValidationResult = response.json().await
        .context("Failed to parse validation result")?;

    debug!(
        cvm_api_url = cvm_api_url,
        version = migration.version,
        valid = validation_result.valid,
        "Migration validation completed"
    );

    Ok(validation_result)
}

/// Get migration history from CVM
pub async fn get_cvm_migration_history(
    cvm_api_url: &str,
    limit: Option<u32>,
) -> Result<Vec<MigrationHistoryEntry>> {
    debug!(
        cvm_api_url = cvm_api_url,
        limit = ?limit,
        "Getting CVM migration history"
    );

    let history_url = if let Some(limit) = limit {
        format!("{}/migrations/history?limit={}", cvm_api_url, limit)
    } else {
        format!("{}/migrations/history", cvm_api_url)
    };

    let client = reqwest::Client::new();
    let response = client
        .get(&history_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to get migration history")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to get migration history: {}", error_text));
    }

    let history: Vec<MigrationHistoryEntry> = response.json().await
        .context("Failed to parse migration history")?;

    debug!(
        cvm_api_url = cvm_api_url,
        entry_count = history.len(),
        "Retrieved CVM migration history"
    );

    Ok(history)
}

/// Rollback migration on CVM
pub async fn rollback_cvm_migration(
    cvm_api_url: &str,
    version: &str,
) -> Result<()> {
    info!(
        cvm_api_url = cvm_api_url,
        version = version,
        "Rolling back migration on CVM"
    );

    let rollback_url = format!("{}/migrations/{}/rollback", cvm_api_url, version);

    let client = reqwest::Client::new();
    let response = client
        .post(&rollback_url)
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to rollback migration")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to rollback migration: {}", error_text));
    }

    info!(
        cvm_api_url = cvm_api_url,
        version = version,
        "Migration rolled back successfully"
    );

    Ok(())
}

/// CVM migration status
#[derive(Debug, Deserialize)]
pub struct CvmMigrationStatus {
    pub status: String,
    pub current_version: Option<String>,
    pub applied_migrations: Vec<AppliedMigrationInfo>,
    pub pending_migrations: Vec<Migration>,
    pub last_migration: Option<chrono::DateTime<chrono::Utc>>,
}

/// Applied migration information
#[derive(Debug, Deserialize)]
pub struct AppliedMigrationInfo {
    pub version: String,
    pub name: String,
    pub applied_at: chrono::DateTime<chrono::Utc>,
    pub checksum: String,
    pub execution_time_ms: Option<u64>,
}

/// Migration validation result
#[derive(Debug, Deserialize)]
pub struct MigrationValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub checksum_match: bool,
    pub syntax_valid: bool,
}

/// Migration history entry
#[derive(Debug, Deserialize)]
pub struct MigrationHistoryEntry {
    pub version: String,
    pub name: String,
    pub action: String, // "applied", "rolled_back", "failed"
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub execution_time_ms: Option<u64>,
    pub error_message: Option<String>,
}

/// Migration batch operation
pub async fn apply_migration_batch(
    cvm_api_url: &str,
    migrations: Vec<Migration>,
) -> Result<BatchMigrationResult> {
    info!(
        cvm_api_url = cvm_api_url,
        migration_count = migrations.len(),
        "Applying migration batch"
    );

    let batch_url = format!("{}/migrations/batch", cvm_api_url);

    #[derive(Serialize)]
    struct BatchRequest {
        migrations: Vec<Migration>,
    }

    let batch_request = BatchRequest { migrations };

    let client = reqwest::Client::new();
    let response = client
        .post(&batch_url)
        .header("Content-Type", "application/json")
        .json(&batch_request)
        .send()
        .await
        .context("Failed to apply migration batch")?;

    if !response.status().is_success() {
        let error_text = response.text().await
            .context("Failed to read error response")?;
        return Err(anyhow::anyhow!("Failed to apply migration batch: {}", error_text));
    }

    let result: BatchMigrationResult = response.json().await
        .context("Failed to parse batch result")?;

    info!(
        cvm_api_url = cvm_api_url,
        success_count = result.successful_migrations.len(),
        failed_count = result.failed_migrations.len(),
        "Migration batch completed"
    );

    Ok(result)
}

/// Batch migration result
#[derive(Debug, Deserialize)]
pub struct BatchMigrationResult {
    pub successful_migrations: Vec<String>,
    pub failed_migrations: Vec<FailedMigration>,
    pub total_execution_time_ms: u64,
}

/// Failed migration information
#[derive(Debug, Deserialize)]
pub struct FailedMigration {
    pub version: String,
    pub error_message: String,
}
