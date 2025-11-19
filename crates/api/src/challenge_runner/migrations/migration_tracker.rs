//! Migration tracking and status management

use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use tracing::{debug, info};

use super::core::AppliedMigration;

/// Get applied migrations for a schema
pub async fn get_applied_migrations(pool: &PgPool, schema_name: &str) -> Result<Vec<AppliedMigration>> {
    debug!(schema = schema_name, "Getting applied migrations");

    let rows = sqlx::query!(
        r#"
        SELECT version, name, applied_at, checksum
        FROM {}.schema_migrations
        ORDER BY applied_at ASC
        "#,
        schema_name
    )
    .fetch_all(pool)
    .await
    .context("Failed to get applied migrations")?;

    let migrations = rows
        .into_iter()
        .map(|row| AppliedMigration {
            version: row.version,
            name: row.name,
            applied_at: row.applied_at,
            checksum: row.checksum,
        })
        .collect();

    debug!(
        schema = schema_name,
        count = migrations.len(),
        "Retrieved applied migrations"
    );

    Ok(migrations)
}

/// Record migration as applied
pub async fn record_migration(
    pool: &PgPool,
    schema_name: &str,
    migration: &super::core::Migration,
) -> Result<()> {
    debug!(
        schema = schema_name,
        version = migration.version,
        "Recording migration as applied"
    );

    sqlx::query!(
        r#"
        INSERT INTO {}.schema_migrations (version, name, checksum, applied_at)
        VALUES ($1, $2, $3, NOW())
        ON CONFLICT (version) DO NOTHING
        "#,
        schema_name,
        migration.version,
        migration.name,
        migration.checksum
    )
    .execute(pool)
    .await
    .context("Failed to record migration")?;

    info!(
        schema = schema_name,
        version = migration.version,
        "Migration recorded as applied"
    );

    Ok(())
}

/// Remove migration record (for rollback scenarios)
pub async fn remove_migration_record(
    pool: &PgPool,
    schema_name: &str,
    version: &str,
) -> Result<()> {
    debug!(
        schema = schema_name,
        version = version,
        "Removing migration record"
    );

    let result = sqlx::query!(
        r#"
        DELETE FROM {}.schema_migrations
        WHERE version = $1
        "#,
        schema_name,
        version
    )
    .execute(pool)
    .await
    .context("Failed to remove migration record")?;

    if result.rows_affected() > 0 {
        info!(
            schema = schema_name,
            version = version,
            "Migration record removed"
        );
    } else {
        debug!(
            schema = schema_name,
            version = version,
            "Migration record not found for removal"
        );
    }

    Ok(())
}

/// Check if migration is applied
pub async fn is_migration_applied(
    pool: &PgPool,
    schema_name: &str,
    version: &str,
) -> Result<bool> {
    debug!(
        schema = schema_name,
        version = version,
        "Checking if migration is applied"
    );

    let result = sqlx::query!(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM {}.schema_migrations 
            WHERE version = $1
        ) as applied
        "#,
        schema_name,
        version
    )
    .fetch_one(pool)
    .await
    .context("Failed to check migration status")?;

    Ok(result.applied.unwrap_or(false))
}

/// Get latest applied migration version
pub async fn get_latest_migration_version(pool: &PgPool, schema_name: &str) -> Result<Option<String>> {
    debug!(schema = schema_name, "Getting latest migration version");

    let result = sqlx::query!(
        r#"
        SELECT version
        FROM {}.schema_migrations
        ORDER BY applied_at DESC
        LIMIT 1
        "#,
        schema_name
    )
    .fetch_optional(pool)
    .await
    .context("Failed to get latest migration version")?;

    let version = result.map(|row| row.version);
    debug!(
        schema = schema_name,
        version = ?version,
        "Retrieved latest migration version"
    );

    Ok(version)
}

/// Get migration count
pub async fn get_migration_count(pool: &PgPool, schema_name: &str) -> Result<i64> {
    debug!(schema = schema_name, "Getting migration count");

    let result = sqlx::query!(
        r#"
        SELECT COUNT(*) as count
        FROM {}.schema_migrations
        "#,
        schema_name
    )
    .fetch_one(pool)
    .await
    .context("Failed to get migration count")?;

    let count = result.count.unwrap_or(0);
    debug!(
        schema = schema_name,
        count = count,
        "Retrieved migration count"
    );

    Ok(count)
}

/// Update migration checksum (for migration file changes)
pub async fn update_migration_checksum(
    pool: &PgPool,
    schema_name: &str,
    version: &str,
    new_checksum: &str,
) -> Result<()> {
    debug!(
        schema = schema_name,
        version = version,
        "Updating migration checksum"
    );

    let result = sqlx::query!(
        r#"
        UPDATE {}.schema_migrations
        SET checksum = $1
        WHERE version = $2
        "#,
        schema_name,
        new_checksum,
        version
    )
    .execute(pool)
    .await
    .context("Failed to update migration checksum")?;

    if result.rows_affected() > 0 {
        info!(
            schema = schema_name,
            version = version,
            "Migration checksum updated"
        );
    } else {
        warn!(
            schema = schema_name,
            version = version,
            "Migration not found for checksum update"
        );
    }

    Ok(())
}
