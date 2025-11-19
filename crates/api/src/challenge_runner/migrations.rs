//! Refactored migrations module with organized components

pub mod core;
pub mod schema_manager;
pub mod migration_tracker;
pub mod migration_executor;
pub mod migration_filter;
pub mod migration_loader;
pub mod backup_manager;

use sqlx::PgPool;
use anyhow::Result;

pub use core::{
    Migration, AppliedMigration, MigrationStatus, MigrationInfo, MigrationRunner,
};
pub use backup_manager::BackupInfo;

/// Create migration runner with all components
pub fn create_migration_runner(pool: PgPool) -> MigrationRunner {
    MigrationRunner::new(pool)
}

/// Initialize migration system
pub async fn initialize_migrations(pool: &PgPool) -> Result<()> {
    // Ensure migrations directory exists
    std::fs::create_dir_all("migrations")
        .map_err(|e| anyhow::anyhow!("Failed to create migrations directory: {}", e))?;

    info!("Migration system initialized");
    Ok(())
}

/// Migration utilities
pub struct MigrationUtils;

impl MigrationUtils {
    /// Create a new migration file
    pub async fn create_migration(
        schema_name: &str,
        version: &str,
        name: &str,
    ) -> Result<std::path::PathBuf> {
        let output_dir = std::path::Path::new("migrations");
        backup_manager::create_migration_template(schema_name, version, name, output_dir)
    }

    /// Validate all migration files
    pub async fn validate_migrations(schema_name: &str) -> Result<()> {
        let migrations_dir = std::path::Path::new("migrations").join(schema_name);
        
        if !migrations_dir.exists() {
            return Err(anyhow::anyhow!("Migrations directory not found"));
        }

        for entry in std::fs::read_dir(&migrations_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                migration_loader::validate_migration_file(&path)?;
            }
        }

        info!("All migration files validated for schema: {}", schema_name);
        Ok(())
    }

    /// Get migration statistics
    pub async fn get_migration_stats(pool: &PgPool, schema_name: &str) -> Result<MigrationStats> {
        let runner = MigrationRunner::new(pool.clone());
        let status = runner.get_migration_status(schema_name).await?;
        
        let total_available = migration_loader::load_available_migrations(schema_name).await?.len();
        let applied_count = status.applied_migrations.len();
        let pending_count = status.pending_migrations.len();
        
        let last_applied = status.applied_migrations.last().map(|m| m.applied_at);
        
        Ok(MigrationStats {
            schema_name: schema_name.to_string(),
            total_available: total_available as u64,
            applied_count: applied_count as u64,
            pending_count: pending_count as u64,
            last_applied,
            current_version: status.current_version,
        })
    }
}

/// Migration statistics
#[derive(Debug, serde::Serialize)]
pub struct MigrationStats {
    pub schema_name: String,
    pub total_available: u64,
    pub applied_count: u64,
    pub pending_count: u64,
    pub last_applied: Option<chrono::DateTime<chrono::Utc>>,
    pub current_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_version_parsing() {
        // Test version parsing logic
        assert!(migration_filter::parse_semver("1.0.0").is_ok());
        assert!(migration_filter::parse_semver("1.0").is_err());
    }

    #[test]
    fn test_filename_parsing() {
        assert!(migration_loader::parse_migration_filename("001_create_users.sql").is_ok());
        assert!(migration_loader::parse_migration_filename("invalid.txt").is_err());
    }
}
