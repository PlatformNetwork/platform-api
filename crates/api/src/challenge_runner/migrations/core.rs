//! Core migration functionality and types

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use tracing::{error, info, warn};

/// Migration to be applied
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Migration {
    pub version: String,
    pub name: String,
    pub sql: String,
    pub checksum: String,
}

/// Applied migration information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedMigration {
    pub version: String,
    pub name: String,
    pub applied_at: chrono::DateTime<chrono::Utc>,
    pub checksum: String,
}

/// Migration status information
#[derive(Debug, Clone, Serialize)]
pub struct MigrationStatus {
    pub schema_name: String,
    pub current_version: Option<String>,
    pub pending_migrations: Vec<Migration>,
    pub applied_migrations: Vec<AppliedMigration>,
    pub last_applied: Option<chrono::DateTime<chrono::Utc>>,
}

/// Migration information for API responses
#[derive(Debug, Clone, Serialize)]
pub struct MigrationInfo {
    pub version: String,
    pub name: String,
    pub applied_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: String, // "pending", "applied", "failed"
}

impl From<AppliedMigration> for MigrationInfo {
    fn from(migration: AppliedMigration) -> Self {
        Self {
            version: migration.version,
            name: migration.name,
            applied_at: Some(migration.applied_at),
            status: "applied".to_string(),
        }
    }
}

/// Core migration runner
#[derive(Clone)]
pub struct MigrationRunner {
    pool: PgPool,
}

impl MigrationRunner {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Apply migrations to a challenge schema
    pub async fn apply_migrations(
        &self,
        schema_name: &str,
        migrations: Vec<Migration>,
    ) -> Result<Vec<String>> {
        info!(schema = schema_name, "Applying migrations");

        // Ensure schema exists
        super::schema_manager::ensure_schema(&self.pool, schema_name).await?;

        // Ensure migrations table exists
        super::schema_manager::ensure_migrations_table(&self.pool, schema_name).await?;

        // Get applied migrations
        let applied_migrations = super::migration_tracker::get_applied_migrations(&self.pool, schema_name).await?;

        // Filter pending migrations
        let pending_migrations = super::migration_filter::filter_pending_migrations(migrations, applied_migrations)?;

        if pending_migrations.is_empty() {
            info!("No pending migrations for schema: {}", schema_name);
            return Ok(vec![]);
        }

        // Apply migrations in order
        let mut applied_versions = Vec::new();
        for migration in pending_migrations {
            info!(
                schema = schema_name,
                version = migration.version,
                name = migration.name,
                "Applying migration"
            );

            super::migration_executor::execute_migration(&self.pool, schema_name, &migration).await?;
            applied_versions.push(migration.version.clone());

            info!(
                schema = schema_name,
                version = migration.version,
                "Migration applied successfully"
            );
        }

        Ok(applied_versions)
    }

    /// Get migration status for a schema
    pub async fn get_migration_status(&self, schema_name: &str) -> Result<MigrationStatus> {
        // Ensure schema exists
        super::schema_manager::ensure_schema(&self.pool, schema_name).await?;

        // Get applied migrations
        let applied_migrations = super::migration_tracker::get_applied_migrations(&self.pool, schema_name).await?;

        // Get all available migrations (this would typically come from a migrations directory)
        let all_migrations = super::migration_loader::load_available_migrations(schema_name).await?;

        // Filter pending migrations
        let pending_migrations = super::migration_filter::filter_pending_migrations(all_migrations, applied_migrations.clone())?;

        let current_version = applied_migrations.last().map(|m| m.version.clone());
        let last_applied = applied_migrations.last().map(|m| m.applied_at);

        Ok(MigrationStatus {
            schema_name: schema_name.to_string(),
            current_version,
            pending_migrations,
            applied_migrations,
            last_applied,
        })
    }

    /// Validate migration checksums
    pub async fn validate_migration_checksums(&self, schema_name: &str) -> Result<Vec<String>> {
        info!(schema = schema_name, "Validating migration checksums");

        let applied_migrations = super::migration_tracker::get_applied_migrations(&self.pool, schema_name).await?;
        let mut invalid_migrations = Vec::new();

        for applied_migration in applied_migrations {
            // Get the original migration file to compare checksums
            if let Ok(original_migration) = super::migration_loader::load_migration_by_version(schema_name, &applied_migration.version).await {
                if original_migration.checksum != applied_migration.checksum {
                    warn!(
                        schema = schema_name,
                        version = applied_migration.version,
                        "Migration checksum mismatch"
                    );
                    invalid_migrations.push(applied_migration.version);
                }
            } else {
                warn!(
                    schema = schema_name,
                    version = applied_migration.version,
                    "Original migration file not found"
                );
                invalid_migrations.push(applied_migration.version);
            }
        }

        if invalid_migrations.is_empty() {
            info!("All migration checksums are valid for schema: {}", schema_name);
        } else {
            error!(
                schema = schema_name,
                count = invalid_migrations.len(),
                "Found invalid migration checksums"
            );
        }

        Ok(invalid_migrations)
    }

    /// Create migration backup
    pub async fn create_migration_backup(&self, schema_name: &str) -> Result<String> {
        info!(schema = schema_name, "Creating migration backup");

        let backup_id = format!("backup_{}_{}", schema_name, chrono::Utc::now().timestamp());
        
        // Create backup using pg_dump
        super::backup_manager::create_schema_backup(&self.pool, schema_name, &backup_id).await?;

        info!(
            schema = schema_name,
            backup_id = backup_id,
            "Migration backup created successfully"
        );

        Ok(backup_id)
    }

    /// Restore from migration backup
    pub async fn restore_migration_backup(&self, schema_name: &str, backup_id: &str) -> Result<()> {
        info!(
            schema = schema_name,
            backup_id = backup_id,
            "Restoring from migration backup"
        );

        super::backup_manager::restore_schema_backup(&self.pool, schema_name, backup_id).await?;

        info!(
            schema = schema_name,
            backup_id = backup_id,
            "Migration backup restored successfully"
        );

        Ok(())
    }
}
