//! Migration backup and restore functionality

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::process::Command;
use tempfile::NamedTempFile;
use tracing::{debug, error, info, warn};

/// Create schema backup using pg_dump
pub async fn create_schema_backup(
    pool: &PgPool,
    schema_name: &str,
    backup_id: &str,
) -> Result<()> {
    info!(
        schema = schema_name,
        backup_id = backup_id,
        "Creating schema backup"
    );

    // Get database connection info
    let db_url = get_database_url(pool).await?;

    // Create backup directory if it doesn't exist
    let backup_dir = std::path::Path::new("backups");
    std::fs::create_dir_all(backup_dir)
        .context("Failed to create backup directory")?;

    let backup_file = backup_dir.join(format!("{}.sql", backup_id));

    // Execute pg_dump command
    let output = Command::new("pg_dump")
        .arg(&db_url)
        .arg("--schema")
        .arg(schema_name)
        .arg("--no-owner")
        .arg("--no-privileges")
        .arg("--clean")
        .arg("--if-exists")
        .arg("--verbose")
        .arg("--file")
        .arg(&backup_file)
        .output()
        .context("Failed to execute pg_dump command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(
            schema = schema_name,
            backup_id = backup_id,
            stderr = %stderr,
            "pg_dump backup failed"
        );
        return Err(anyhow::anyhow!("pg_dump backup failed: {}", stderr));
    }

    // Verify backup file was created
    if !backup_file.exists() {
        return Err(anyhow::anyhow!("Backup file was not created"));
    }

    let file_size = std::fs::metadata(&backup_file)
        .context("Failed to get backup file metadata")?
        .len();

    info!(
        schema = schema_name,
        backup_id = backup_id,
        file_size = file_size,
        backup_file = ?backup_file,
        "Schema backup created successfully"
    );

    Ok(())
}

/// Restore schema from backup file
pub async fn restore_schema_backup(
    pool: &PgPool,
    schema_name: &str,
    backup_id: &str,
) -> Result<()> {
    info!(
        schema = schema_name,
        backup_id = backup_id,
        "Restoring schema from backup"
    );

    let backup_dir = std::path::Path::new("backups");
    let backup_file = backup_dir.join(format!("{}.sql", backup_id));

    if !backup_file.exists() {
        return Err(anyhow::anyhow!("Backup file not found: {:?}", backup_file));
    }

    // Get database connection info
    let db_url = get_database_url(pool).await?;

    // First, drop the existing schema
    super::schema_manager::drop_schema(pool, schema_name).await?;

    // Execute psql restore command
    let output = Command::new("psql")
        .arg(&db_url)
        .arg("-v")
        .arg("ON_ERROR_STOP=1")
        .arg("-f")
        .arg(&backup_file)
        .output()
        .context("Failed to execute psql restore command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        error!(
            schema = schema_name,
            backup_id = backup_id,
            stderr = %stderr,
            stdout = %stdout,
            "psql restore failed"
        );

        return Err(anyhow::anyhow!("psql restore failed: {}", stderr));
    }

    info!(
        schema = schema_name,
        backup_id = backup_id,
        "Schema backup restored successfully"
    );

    Ok(())
}

/// List available backups
pub async fn list_backups() -> Result<Vec<BackupInfo>> {
    debug!("Listing available backups");

    let backup_dir = std::path::Path::new("backups");
    
    if !backup_dir.exists() {
        return Ok(vec![]);
    }

    let mut backups = Vec::new();

    for entry in std::fs::read_dir(backup_dir)
        .context("Failed to read backup directory")? 
    {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            let backup_info = get_backup_info(&path).await?;
            backups.push(backup_info);
        }
    }

    // Sort by creation time (newest first)
    backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    info!(backup_count = backups.len(), "Listed available backups");
    Ok(backups)
}

/// Get backup information
async fn get_backup_info(backup_file: &std::path::Path) -> Result<BackupInfo> {
    let filename = backup_file.file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid backup filename"))?;

    let metadata = std::fs::metadata(backup_file)
        .context("Failed to get backup file metadata")?;

    let created_at = metadata.created()
        .or_else(|_| metadata.modified())
        .map(|time| chrono::DateTime::<chrono::Utc>::from(time))
        .unwrap_or_else(|_| chrono::Utc::now());

    let file_size = metadata.len();

    // Try to extract schema name from backup ID
    let schema_name = extract_schema_from_backup_id(filename);

    Ok(BackupInfo {
        backup_id: filename.to_string(),
        schema_name,
        file_path: backup_file.to_path_buf(),
        file_size,
        created_at,
    })
}

/// Extract schema name from backup ID
fn extract_schema_from_backup_id(backup_id: &str) -> String {
    // Expected format: "backup_schema_name_timestamp"
    let parts: Vec<&str> = backup_id.split('_').collect();
    
    if parts.len() >= 3 {
        parts[1].to_string() // Second part should be schema name
    } else {
        "unknown".to_string()
    }
}

/// Delete old backups (cleanup)
pub async fn cleanup_old_backups(retention_days: u32) -> Result<usize> {
    info!(retention_days = retention_days, "Cleaning up old backups");

    let backups = list_backups().await?;
    let cutoff_time = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);

    let mut deleted_count = 0;

    for backup in backups {
        if backup.created_at < cutoff_time {
            warn!(
                backup_id = backup.backup_id,
                created_at = ?backup.created_at,
                "Deleting old backup"
            );

            if let Err(e) = std::fs::remove_file(&backup.file_path) {
                error!(
                    backup_id = backup.backup_id,
                    error = %e,
                    "Failed to delete backup file"
                );
            } else {
                deleted_count += 1;
            }
        }
    }

    info!(
        deleted_count = deleted_count,
        retention_days = retention_days,
        "Cleanup completed"
    );

    Ok(deleted_count)
}

/// Verify backup integrity
pub async fn verify_backup_integrity(backup_id: &str) -> Result<bool> {
    debug!(
        backup_id = backup_id,
        "Verifying backup integrity"
    );

    let backup_dir = std::path::Path::new("backups");
    let backup_file = backup_dir.join(format!("{}.sql", backup_id));

    if !backup_file.exists() {
        return Ok(false);
    }

    // Check file size
    let metadata = std::fs::metadata(&backup_file)
        .context("Failed to get backup file metadata")?;
    
    if metadata.len() == 0 {
        warn!(
            backup_id = backup_id,
            "Backup file is empty"
        );
        return Ok(false);
    }

    // Try to parse the SQL file (basic check)
    let content = std::fs::read_to_string(&backup_file)
        .context("Failed to read backup file")?;

    // Check if it contains basic SQL structure
    if !content.contains("CREATE") && !content.contains("ALTER") && !content.contains("INSERT") {
        warn!(
            backup_id = backup_id,
            "Backup file appears to be invalid (no SQL statements found)"
        );
        return Ok(false);
    }

    debug!(
        backup_id = backup_id,
        file_size = metadata.len(),
        "Backup integrity verified"
    );

    Ok(true)
}

/// Get database URL from pool (simplified implementation)
async fn get_database_url(pool: &PgPool) -> Result<String> {
    // In a real implementation, you'd extract this from the pool configuration
    Ok("postgresql://localhost/platform_api".to_string())
}

/// Backup information structure
#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub backup_id: String,
    pub schema_name: String,
    pub file_path: std::path::PathBuf,
    pub file_size: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Create differential backup (only changes since last backup)
pub async fn create_differential_backup(
    pool: &PgPool,
    schema_name: &str,
    backup_id: &str,
    since_backup_id: &str,
) -> Result<()> {
    info!(
        schema = schema_name,
        backup_id = backup_id,
        since_backup_id = since_backup_id,
        "Creating differential backup"
    );

    // This is a simplified implementation
    // In practice, you would compare the current state with the previous backup
    // and generate only the differences
    
    // For now, just create a full backup
    create_schema_backup(pool, schema_name, backup_id).await?;

    info!(
        schema = schema_name,
        backup_id = backup_id,
        "Differential backup created (as full backup)"
    );

    Ok(())
}
