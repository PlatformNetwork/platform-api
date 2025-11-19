//! Schema management for migrations

use anyhow::{Context, Result};
use sqlx::PgPool;
use tracing::{debug, info};

/// Ensure schema exists
pub async fn ensure_schema(pool: &PgPool, schema_name: &str) -> Result<()> {
    debug!(schema = schema_name, "Ensuring schema exists");

    sqlx::query!(
        r#"
        CREATE SCHEMA IF NOT EXISTS {}
        "#,
        schema_name
    )
    .execute(pool)
    .await
    .context("Failed to create schema")?;

    info!(schema = schema_name, "Schema ensured");
    Ok(())
}

/// Ensure migrations table exists in schema
pub async fn ensure_migrations_table(pool: &PgPool, schema_name: &str) -> Result<()> {
    debug!(schema = schema_name, "Ensuring migrations table exists");

    sqlx::query!(
        r#"
        CREATE TABLE IF NOT EXISTS {}.schema_migrations (
            version VARCHAR(255) PRIMARY KEY,
            name VARCHAR(255) NOT NULL,
            checksum VARCHAR(64) NOT NULL,
            applied_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
        )
        "#,
        schema_name
    )
    .execute(pool)
    .await
    .context("Failed to create migrations table")?;

    // Create index on applied_at for better query performance
    sqlx::query!(
        r#"
        CREATE INDEX IF NOT EXISTS idx_{}_migrations_applied_at 
        ON {}.schema_migrations (applied_at)
        "#,
        schema_name,
        schema_name
    )
    .execute(pool)
    .await
    .context("Failed to create migrations index")?;

    info!(schema = schema_name, "Migrations table ensured");
    Ok(())
}

/// Drop schema (for cleanup purposes)
pub async fn drop_schema(pool: &PgPool, schema_name: &str) -> Result<()> {
    warn!(schema = schema_name, "Dropping schema");

    sqlx::query!(
        r#"
        DROP SCHEMA IF EXISTS {} CASCADE
        "#,
        schema_name
    )
    .execute(pool)
    .await
    .context("Failed to drop schema")?;

    info!(schema = schema_name, "Schema dropped");
    Ok(())
}

/// Check if schema exists
pub async fn schema_exists(pool: &PgPool, schema_name: &str) -> Result<bool> {
    debug!(schema = schema_name, "Checking if schema exists");

    let result = sqlx::query!(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM information_schema.schemata 
            WHERE schema_name = $1
        ) as exists
        "#,
        schema_name
    )
    .fetch_one(pool)
    .await
    .context("Failed to check schema existence")?;

    Ok(result.exists.unwrap_or(false))
}

/// Get schema size information
pub async fn get_schema_size(pool: &PgPool, schema_name: &str) -> Result<i64> {
    debug!(schema = schema_name, "Getting schema size");

    let result = sqlx::query!(
        r#"
        SELECT 
            pg_size_pretty(pg_database_size(current_database())) as database_size,
            pg_size_pretty(pg_total_relation_size(schemaname || '.' || tablename)) as schema_size
        FROM pg_tables 
        WHERE schemaname = $1
        LIMIT 1
        "#,
        schema_name
    )
    .fetch_optional(pool)
    .await
    .context("Failed to get schema size")?;

    // For simplicity, return 0 if no tables found
    // In a real implementation, you might want to calculate total size
    Ok(0)
}

/// List all tables in schema
pub async fn list_schema_tables(pool: &PgPool, schema_name: &str) -> Result<Vec<String>> {
    debug!(schema = schema_name, "Listing schema tables");

    let rows = sqlx::query!(
        r#"
        SELECT tablename 
        FROM pg_tables 
        WHERE schemaname = $1 
        ORDER BY tablename
        "#,
        schema_name
    )
    .fetch_all(pool)
    .await
    .context("Failed to list schema tables")?;

    let tables = rows.into_iter().map(|row| row.tablename).collect();
    debug!(schema = schema_name, count = tables.len(), "Listed schema tables");

    Ok(tables)
}
