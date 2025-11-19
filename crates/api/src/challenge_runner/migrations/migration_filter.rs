//! Migration filtering and ordering logic

use anyhow::{Context, Result};
use std::collections::HashSet;
use tracing::{debug, info, warn};

use super::core::{AppliedMigration, Migration};

/// Filter pending migrations (not yet applied)
pub fn filter_pending_migrations(
    all_migrations: Vec<Migration>,
    applied_migrations: Vec<AppliedMigration>,
) -> Result<Vec<Migration>> {
    debug!(
        total_migrations = all_migrations.len(),
        applied_migrations = applied_migrations.len(),
        "Filtering pending migrations"
    );

    // Create set of applied versions for quick lookup
    let applied_versions: HashSet<String> = applied_migrations
        .into_iter()
        .map(|m| m.version)
        .collect();

    // Filter out already applied migrations
    let pending_migrations: Vec<Migration> = all_migrations
        .into_iter()
        .filter(|m| !applied_versions.contains(&m.version))
        .collect();

    // Sort migrations by version (assuming semantic versioning)
    let sorted_migrations = sort_migrations_by_version(pending_migrations)?;

    info!(
        pending_count = sorted_migrations.len(),
        "Filtered pending migrations"
    );

    Ok(sorted_migrations)
}

/// Sort migrations by version using semantic versioning
fn sort_migrations_by_version(mut migrations: Vec<Migration>) -> Result<Vec<Migration>> {
    migrations.sort_by(|a, b| {
        compare_versions(&a.version, &b.version)
    });

    // Validate that versions are unique
    let mut seen_versions = HashSet::new();
    for migration in &migrations {
        if seen_versions.contains(&migration.version) {
            warn!("Duplicate migration version found: {}", migration.version);
        }
        seen_versions.insert(migration.version.clone());
    }

    Ok(migrations)
}

/// Compare two version strings
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    // Try to parse as semantic version
    if let (Ok(a_parsed), Ok(b_parsed)) = (parse_semver(a), parse_semver(b)) {
        return a_parsed.cmp(&b_parsed);
    }

    // Fallback to string comparison
    a.cmp(b)
}

/// Parse semantic version (simplified)
fn parse_semver(version: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!("Invalid semantic version format"));
    }

    let major = parts[0].parse::<u64>()
        .context("Invalid major version")?;
    let minor = parts[1].parse::<u64>()
        .context("Invalid minor version")?;
    let patch = parts[2].parse::<u64>()
        .context("Invalid patch version")?;

    Ok((major, minor, patch))
}

/// Filter migrations by type
pub fn filter_migrations_by_type(
    migrations: Vec<Migration>,
    migration_type: &str,
) -> Vec<Migration> {
    debug!(
        total_migrations = migrations.len(),
        migration_type = migration_type,
        "Filtering migrations by type"
    );

    migrations
        .into_iter()
        .filter(|m| {
            // Check if migration name contains the type
            // This is a simple implementation - could be made more sophisticated
            m.name.to_lowercase().contains(&migration_type.to_lowercase())
        })
        .collect()
}

/// Filter migrations by date range
pub fn filter_migrations_by_date_range(
    migrations: Vec<Migration>,
    start_date: Option<chrono::DateTime<chrono::Utc>>,
    end_date: Option<chrono::DateTime<chrono::Utc>>,
) -> Vec<Migration> {
    debug!(
        total_migrations = migrations.len(),
        start_date = ?start_date,
        end_date = ?end_date,
        "Filtering migrations by date range"
    );

    migrations
        .into_iter()
        .filter(|m| {
            // This would require migration timestamps
            // For now, return all migrations
            true
        })
        .collect()
}

/// Get migration dependencies
pub fn get_migration_dependencies(migration: &Migration) -> Vec<String> {
    // Parse migration SQL to find dependencies
    // This is a simplified implementation
    let mut dependencies = Vec::new();

    // Look for common dependency patterns in SQL comments
    let lines: Vec<&str> = migration.sql.lines().collect();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("-- depends:") {
            let deps_part = trimmed.strip_prefix("-- depends:").unwrap().trim();
            dependencies.extend(deps_part.split(',').map(|s| s.trim().to_string()));
        }
    }

    dependencies
}

/// Filter migrations that can be applied (dependencies satisfied)
pub fn filter_applicable_migrations(
    migrations: Vec<Migration>,
    applied_versions: &HashSet<String>,
) -> Vec<Migration> {
    debug!(
        total_migrations = migrations.len(),
        applied_count = applied_versions.len(),
        "Filtering applicable migrations"
    );

    migrations
        .into_iter()
        .filter(|m| {
            let dependencies = get_migration_dependencies(m);
            dependencies.iter().all(|dep| applied_versions.contains(dep))
        })
        .collect()
}

/// Validate migration order
pub fn validate_migration_order(migrations: &[Migration]) -> Result<()> {
    debug!(migration_count = migrations.len(), "Validating migration order");

    for (i, migration) in migrations.iter().enumerate() {
        let dependencies = get_migration_dependencies(migration);
        
        for dependency in dependencies {
            // Check if dependency appears before this migration
            let dep_index = migrations.iter().position(|m| m.version == dependency);
            
            if let Some(dep_idx) = dep_index {
                if dep_idx >= i {
                    return Err(anyhow::anyhow!(
                        "Migration {} depends on {} which appears after it",
                        migration.version,
                        dependency
                    ));
                }
            } else {
                warn!(
                    migration = migration.version,
                    dependency = dependency,
                    "Migration dependency not found in migration list"
                );
            }
        }
    }

    debug!("Migration order validation passed");
    Ok(())
}

/// Group migrations by category
pub fn group_migrations_by_category(migrations: Vec<Migration>) -> std::collections::HashMap<String, Vec<Migration>> {
    debug!(migration_count = migrations.len(), "Grouping migrations by category");

    let mut categories: std::collections::HashMap<String, Vec<Migration>> = std::collections::HashMap::new();

    for migration in migrations {
        let category = extract_migration_category(&migration);
        categories.entry(category).or_insert_with(Vec::new).push(migration);
    }

    debug!(
        category_count = categories.len(),
        "Grouped migrations by category"
    );

    categories
}

/// Extract category from migration name
fn extract_migration_category(migration: &Migration) -> String {
    // Simple category extraction based on naming conventions
    let name_lower = migration.name.to_lowercase();
    
    if name_lower.contains("schema") {
        "schema".to_string()
    } else if name_lower.contains("table") {
        "table".to_string()
    } else if name_lower.contains("index") {
        "index".to_string()
    } else if name_lower.contains("data") {
        "data".to_string()
    } else if name_lower.contains("function") || name_lower.contains("proc") {
        "function".to_string()
    } else if name_lower.contains("trigger") {
        "trigger".to_string()
    } else {
        "other".to_string()
    }
}
