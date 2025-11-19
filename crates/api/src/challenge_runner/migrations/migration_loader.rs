//! Migration loading from files and directories

use anyhow::{Context, Result};
use std::path::Path;
use std::fs;
use tracing::{debug, info, warn};

use super::core::Migration;

/// Load available migrations from directory
pub async fn load_available_migrations(schema_name: &str) -> Result<Vec<Migration>> {
    debug!(schema = schema_name, "Loading available migrations");

    let migrations_dir = Path::new("migrations").join(schema_name);
    
    if !migrations_dir.exists() {
        warn!(
            schema = schema_name,
            dir = ?migrations_dir,
            "Migrations directory not found"
        );
        return Ok(vec![]);
    }

    let mut migrations = Vec::new();

    // Read migration files
    for entry in fs::read_dir(&migrations_dir)
        .context("Failed to read migrations directory")? 
    {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            let migration = load_migration_from_file(&path).await?;
            migrations.push(migration);
        }
    }

    // Sort migrations by filename (which should include version)
    migrations.sort_by(|a, b| a.version.cmp(&b.version));

    info!(
        schema = schema_name,
        count = migrations.len(),
        "Loaded available migrations"
    );

    Ok(migrations)
}

/// Load migration from file
pub async fn load_migration_from_file(file_path: &Path) -> Result<Migration> {
    debug!(file_path = ?file_path, "Loading migration from file");

    let filename = file_path.file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid filename"))?;

    // Extract version and name from filename
    // Expected format: "001_create_users.sql" or "20231201_1200_initial.sql"
    let (version, name) = parse_migration_filename(filename)?;

    // Read SQL content
    let sql = fs::read_to_string(file_path)
        .context("Failed to read migration file")?;

    // Generate checksum
    let checksum = super::migration_executor::generate_checksum(&sql);

    Ok(Migration {
        version,
        name,
        sql,
        checksum,
    })
}

/// Parse migration filename to extract version and name
fn parse_migration_filename(filename: &str) -> Result<(String, String)> {
    // Remove .sql extension
    let base_name = filename.strip_suffix(".sql")
        .ok_or_else(|| anyhow::anyhow!("File must have .sql extension"))?;

    // Split on first underscore to separate version from name
    let parts: Vec<&str> = base_name.splitn(2, '_').collect();
    
    if parts.len() != 2 {
        return Err(anyhow::anyhow!(
            "Filename must be in format: 'version_name.sql', got: {}",
            filename
        ));
    }

    let version = parts[0].to_string();
    let name = parts[1].replace('_', " ");

    // Validate version format
    if version.is_empty() {
        return Err(anyhow::anyhow!("Version cannot be empty"));
    }

    Ok((version, name))
}

/// Load migration by version
pub async fn load_migration_by_version(schema_name: &str, version: &str) -> Result<Migration> {
    debug!(
        schema = schema_name,
        version = version,
        "Loading migration by version"
    );

    let migrations_dir = Path::new("migrations").join(schema_name);
    
    // Look for file matching version
    for entry in fs::read_dir(&migrations_dir)
        .context("Failed to read migrations directory")? 
    {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow::anyhow!("Invalid filename"))?;

            if filename.starts_with(&format!("{}_", version)) {
                return load_migration_from_file(&path).await;
            }
        }
    }

    Err(anyhow::anyhow!(
        "Migration not found for schema {} version {}",
        schema_name,
        version
    ))
}

/// Load migrations from multiple directories
pub async fn load_migrations_from_paths(paths: &[&Path]) -> Result<Vec<Migration>> {
    debug!(path_count = paths.len(), "Loading migrations from multiple paths");

    let mut all_migrations = Vec::new();

    for path in paths {
        let migrations = load_migrations_from_path(path).await?;
        all_migrations.extend(migrations);
    }

    // Remove duplicates and sort
    all_migrations.sort_by(|a, b| a.version.cmp(&b.version));
    all_migrations.dedup_by(|a, b| a.version == b.version);

    info!(
        total_migrations = all_migrations.len(),
        "Loaded migrations from all paths"
    );

    Ok(all_migrations)
}

/// Load migrations from a single path
async fn load_migrations_from_path(path: &Path) -> Result<Vec<Migration>> {
    debug!(path = ?path, "Loading migrations from path");

    let mut migrations = Vec::new();

    if path.is_dir() {
        // Load from directory
        for entry in fs::read_dir(path)
            .context("Failed to read directory")? 
        {
            let entry = entry.context("Failed to read directory entry")?;
            let entry_path = entry.path();

            if entry_path.is_file() && entry_path.extension().and_then(|s| s.to_str()) == Some("sql") {
                let migration = load_migration_from_file(&entry_path).await?;
                migrations.push(migration);
            }
        }
    } else if path.is_file() {
        // Load single file
        let migration = load_migration_from_file(path).await?;
        migrations.push(migration);
    }

    Ok(migrations)
}

/// Validate migration file format
pub fn validate_migration_file(file_path: &Path) -> Result<()> {
    debug!(file_path = ?file_path, "Validating migration file format");

    let filename = file_path.file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid filename"))?;

    // Check extension
    if !filename.ends_with(".sql") {
        return Err(anyhow::anyhow!("Migration file must have .sql extension"));
    }

    // Check filename format
    parse_migration_filename(filename)?;

    // Check file exists and is readable
    if !file_path.exists() {
        return Err(anyhow::anyhow!("Migration file does not exist"));
    }

    // Check file size (prevent extremely large files)
    let metadata = fs::metadata(file_path)
        .context("Failed to read file metadata")?;
    
    if metadata.len() > 10 * 1024 * 1024 { // 10MB limit
        return Err(anyhow::anyhow!("Migration file is too large (max 10MB)"));
    }

    debug!("Migration file format validation passed");
    Ok(())
}

/// Create migration template
pub fn create_migration_template(
    schema_name: &str,
    version: &str,
    name: &str,
    output_dir: &Path,
) -> Result<PathBuf> {
    info!(
        schema = schema_name,
        version = version,
        name = name,
        "Creating migration template"
    );

    let filename = format!("{}_{}.sql", version, name.replace(' ', "_"));
    let file_path = output_dir.join(schema_name).join(filename);

    // Ensure directory exists
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)
            .context("Failed to create migration directory")?;
    }

    // Create template content
    let template = format!(
        r#"-- Migration: {} {}
-- Version: {}
-- Schema: {}
-- Created: {}

-- Add your migration SQL here

-- Example: CREATE TABLE example_table (
--     id SERIAL PRIMARY KEY,
--     name VARCHAR(255) NOT NULL,
--     created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
-- );
"#,
        name,
        version,
        version,
        schema_name,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    fs::write(&file_path, template)
        .context("Failed to write migration template")?;

    info!(
        file_path = ?file_path,
        "Migration template created"
    );

    Ok(file_path)
}

use std::path::PathBuf;
