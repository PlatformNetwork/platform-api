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

/// Applied migration record
#[derive(Debug, Clone)]
pub struct AppliedMigration {
    pub version: String,
    pub name: String,
    pub applied_at: chrono::DateTime<chrono::Utc>,
    pub checksum: String,
}

/// Migration runner for challenge schemas
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
        self.ensure_schema(schema_name).await?;

        // Ensure migrations table exists
        self.ensure_migrations_table(schema_name).await?;

        // Get already applied migrations
        let applied = self.get_applied_migrations(schema_name).await?;
        let applied_map: HashMap<String, AppliedMigration> = applied
            .into_iter()
            .map(|m| (m.version.clone(), m))
            .collect();

        let mut applied_versions = Vec::new();

        // Apply each migration in order
        for migration in migrations {
            if let Some(existing) = applied_map.get(&migration.version) {
                // Check if migration matches
                if existing.checksum != migration.checksum {
                    error!(
                        version = &migration.version,
                        expected = &migration.checksum,
                        actual = &existing.checksum,
                        "Migration checksum mismatch"
                    );
                    return Err(anyhow::anyhow!(
                        "Migration {} has different checksum. Expected {}, got {}",
                        migration.version,
                        migration.checksum,
                        existing.checksum
                    ));
                }

                info!(
                    version = &migration.version,
                    "Migration already applied, skipping"
                );
                continue;
            }

            // Apply the migration
            match self.apply_single_migration(schema_name, &migration).await {
                Ok(_) => {
                    info!(
                        version = &migration.version,
                        name = &migration.name,
                        "✅ Migration applied successfully"
                    );
                    applied_versions.push(migration.version);
                }
                Err(e) => {
                    error!(
                        version = &migration.version,
                        name = &migration.name,
                        error = %e,
                        "❌ Failed to apply migration"
                    );
                    return Err(e).context(format!(
                        "Failed to apply migration {} - {}",
                        migration.version, migration.name
                    ));
                }
            }
        }

        info!(
            schema = schema_name,
            applied_count = applied_versions.len(),
            "Migrations completed"
        );

        Ok(applied_versions)
    }

    /// Ensure schema exists
    pub async fn ensure_schema(&self, schema_name: &str) -> Result<()> {
        // Validate schema name to prevent SQL injection
        if !schema_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(anyhow::anyhow!("Invalid schema name: {}", schema_name));
        }

        // First, check if schema exists by querying information_schema
        let schema_exists = match sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT schema_name 
            FROM information_schema.schemata 
            WHERE schema_name = $1
            "#,
        )
        .bind(schema_name)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(result) => result.is_some(),
            Err(e) => {
                // If query fails, try to create schema anyway
                tracing::warn!(
                    schema = schema_name,
                    error = %e,
                    "Failed to check if schema exists, attempting to create"
                );
                false // Assume schema doesn't exist and try to create
            }
        };

        if schema_exists {
            info!(
                schema = schema_name,
                "Schema already exists, skipping creation"
            );
        } else {
            // Schema doesn't exist, create it
            let create_query = format!("CREATE SCHEMA {} AUTHORIZATION CURRENT_USER", schema_name);

            match sqlx::query(&create_query).execute(&self.pool).await {
                Ok(_) => {
                    info!(schema = schema_name, "Created database schema");
                }
                Err(e) => {
                    // If schema already exists (race condition), that's okay
                    let error_msg = e.to_string();
                    if error_msg.contains("already exists") || error_msg.contains("42P07") {
                        info!(
                            schema = schema_name,
                            "Schema already exists (created concurrently)"
                        );
                    } else {
                        return Err(anyhow::anyhow!(
                            "Failed to create schema '{}': {}",
                            schema_name,
                            e
                        ))
                        .context("Database connection or permission issue?");
                    }
                }
            }
        }

        // Grant permissions (this is idempotent, safe to run even if already granted)
        let grant_query = format!("GRANT ALL ON SCHEMA {} TO CURRENT_USER", schema_name);

        if let Err(e) = sqlx::query(&grant_query).execute(&self.pool).await {
            // Log warning but don't fail - permissions might already be set
            tracing::warn!(
                schema = schema_name,
                error = %e,
                "Failed to grant schema permissions (may already be granted)"
            );
        }

        Ok(())
    }

    /// Ensure migrations table exists in schema
    async fn ensure_migrations_table(&self, schema_name: &str) -> Result<()> {
        let query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {}.schema_migrations (
                version VARCHAR(255) PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                checksum VARCHAR(64) NOT NULL
            )
            "#,
            schema_name
        );

        sqlx::query(&query)
            .execute(&self.pool)
            .await
            .context("Failed to create migrations table")?;

        Ok(())
    }

    /// Get list of applied migrations
    async fn get_applied_migrations(&self, schema_name: &str) -> Result<Vec<AppliedMigration>> {
        let query = format!(
            r#"
            SELECT version, name, applied_at, checksum
            FROM {}.schema_migrations
            ORDER BY version
            "#,
            schema_name
        );

        let rows = sqlx::query(&query)
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

        let migrations = rows
            .into_iter()
            .map(|row| AppliedMigration {
                version: row.get("version"),
                name: row.get("name"),
                applied_at: row.get("applied_at"),
                checksum: row.get("checksum"),
            })
            .collect();

        Ok(migrations)
    }

    /// Apply a single migration
    async fn apply_single_migration(&self, schema_name: &str, migration: &Migration) -> Result<()> {
        info!(
            schema = schema_name,
            version = &migration.version,
            name = &migration.name,
            sql_length = migration.sql.len(),
            "Starting migration application"
        );

        // Start transaction
        info!(schema = schema_name, "Starting database transaction");
        let mut tx = match self.pool.begin().await {
            Ok(t) => {
                info!(schema = schema_name, "✅ Transaction started");
                t
            }
            Err(e) => {
                error!(schema = schema_name, error = %e, "Failed to start transaction");
                return Err(anyhow::anyhow!("Failed to start transaction: {}", e));
            }
        };

        // Set search path
        let set_path = format!("SET search_path TO {}, public", schema_name);
        info!(
            schema = schema_name,
            search_path = &set_path,
            "Setting search_path"
        );
        match sqlx::query(&set_path).execute(&mut *tx).await {
            Ok(_) => {
                info!(schema = schema_name, "✅ search_path set successfully");
            }
            Err(e) => {
                error!(schema = schema_name, error = %e, "Failed to set search_path");
                let _ = tx.rollback().await;
                return Err(anyhow::anyhow!("Failed to set search_path: {}", e));
            }
        }

        // Execute migration SQL
        // Split SQL into individual statements (separated by semicolons)
        // We need to handle multi-line statements correctly and preserve statement integrity
        // Strategy: Split by semicolon, but only when it's at the end of a trimmed line or followed by whitespace
        let mut sql_statements = Vec::new();
        let mut current_statement = String::new();

        // Process line by line to handle multi-line statements correctly
        for line in migration.sql.lines() {
            let trimmed_line = line.trim();

            // Skip empty lines and full-line comments
            if trimmed_line.is_empty() || trimmed_line.starts_with("--") {
                continue;
            }

            // Remove inline comments (but preserve semicolons in comments)
            let line_without_comments = trimmed_line
                .split("--")
                .next()
                .unwrap_or(trimmed_line)
                .trim();

            if line_without_comments.is_empty() {
                continue;
            }

            // Add line to current statement
            if !current_statement.is_empty() {
                current_statement.push('\n');
            }
            current_statement.push_str(line_without_comments);

            // Check if line ends with semicolon (statement terminator)
            if line_without_comments.ends_with(';') {
                // Remove trailing semicolon for this statement
                let statement = current_statement.trim_end_matches(';').trim().to_string();

                if !statement.is_empty() {
                    sql_statements.push(statement);
                }
                current_statement.clear();
            }
        }

        // Add any remaining statement (in case last statement doesn't end with semicolon)
        let remaining = current_statement.trim().to_string();
        if !remaining.is_empty() {
            sql_statements.push(remaining);
        }

        info!(
            schema = schema_name,
            version = &migration.version,
            total_statements = sql_statements.len(),
            sql_preview = &migration.sql.chars().take(200).collect::<String>(),
            "Executing migration SQL (split into {} statements)",
            sql_statements.len()
        );

        for (idx, statement) in sql_statements.iter().enumerate() {
            if statement.trim().is_empty() {
                continue;
            }

            info!(
                schema = schema_name,
                version = &migration.version,
                statement_index = idx + 1,
                total_statements = sql_statements.len(),
                statement_preview = &statement.chars().take(100).collect::<String>(),
                "Executing SQL statement {}/{}",
                idx + 1,
                sql_statements.len()
            );

            match sqlx::query(statement).execute(&mut *tx).await {
                Ok(result) => {
                    info!(
                        schema = schema_name,
                        version = &migration.version,
                        statement_index = idx + 1,
                        rows_affected = result.rows_affected(),
                        "✅ Statement {}/{} executed successfully",
                        idx + 1,
                        sql_statements.len()
                    );
                }
                Err(e) => {
                    let error_details = format!("{}", e);
                    error!(
                        schema = schema_name,
                        version = &migration.version,
                        name = &migration.name,
                        statement_index = idx + 1,
                        total_statements = sql_statements.len(),
                        error = %e,
                        error_details = &error_details,
                        statement = &statement.chars().take(500).collect::<String>(),
                        "❌ Failed to execute SQL statement {}/{}",
                        idx + 1,
                        sql_statements.len()
                    );

                    // Try to extract database error details if available
                    if let Some(db_err) = e.as_database_error() {
                        error!(
                            schema = schema_name,
                            version = &migration.version,
                            db_error_code = ?db_err.code(),
                            db_error_message = db_err.message(),
                            db_error_constraint = db_err.constraint(),
                            db_error_table = db_err.table(),
                            "PostgreSQL error details"
                        );
                    }

                    let _ = tx.rollback().await;
                    return Err(anyhow::anyhow!(
                        "Failed to execute migration SQL statement {}/{} for {} - {}: {}",
                        idx + 1,
                        sql_statements.len(),
                        migration.version,
                        migration.name,
                        e
                    ));
                }
            }
        }

        info!(
            schema = schema_name,
            version = &migration.version,
            total_statements = sql_statements.len(),
            "✅ All migration SQL statements executed successfully"
        );

        // Record migration
        let record_query = format!(
            r#"
            INSERT INTO {}.schema_migrations (version, name, checksum)
            VALUES ($1, $2, $3)
            "#,
            schema_name
        );

        info!(
            schema = schema_name,
            version = &migration.version,
            name = &migration.name,
            "Recording migration in schema_migrations table"
        );
        match sqlx::query(&record_query)
            .bind(&migration.version)
            .bind(&migration.name)
            .bind(&migration.checksum)
            .execute(&mut *tx)
            .await
        {
            Ok(result) => {
                info!(
                    schema = schema_name,
                    version = &migration.version,
                    rows_affected = result.rows_affected(),
                    "✅ Migration recorded in schema_migrations"
                );
            }
            Err(e) => {
                error!(
                    schema = schema_name,
                    version = &migration.version,
                    error = %e,
                    "Failed to record migration in schema_migrations"
                );
                let _ = tx.rollback().await;
                return Err(anyhow::anyhow!(
                    "Failed to record migration {} - {}: {}",
                    migration.version,
                    migration.name,
                    e
                ));
            }
        }

        // Commit transaction
        info!(schema = schema_name, "Committing transaction");
        match tx.commit().await {
            Ok(_) => {
                info!(
                    schema = schema_name,
                    version = &migration.version,
                    "✅ Transaction committed, migration applied successfully"
                );
            }
            Err(e) => {
                error!(
                    schema = schema_name,
                    version = &migration.version,
                    error = %e,
                    "Failed to commit transaction"
                );
                return Err(anyhow::anyhow!(
                    "Failed to commit transaction for migration {} - {}: {}",
                    migration.version,
                    migration.name,
                    e
                ));
            }
        }

        Ok(())
    }

    /// Rollback a migration (if supported)
    pub async fn rollback_migration(&self, schema_name: &str, version: &str) -> Result<()> {
        warn!(
            schema = schema_name,
            version = version,
            "Rollback not implemented yet"
        );

        // Rollback functionality would require storing down migrations
        // Currently not implemented to keep migrations forward-only

        Err(anyhow::anyhow!("Rollback not implemented"))
    }

    /// Get migration status for a schema
    pub async fn get_migration_status(&self, schema_name: &str) -> Result<MigrationStatus> {
        let applied = self.get_applied_migrations(schema_name).await?;

        let latest_version = applied.last().map(|m| m.version.clone());

        Ok(MigrationStatus {
            schema_name: schema_name.to_string(),
            applied_count: applied.len(),
            latest_version,
            migrations: applied.into_iter().map(|m| m.into()).collect(),
        })
    }
}

/// Migration status response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStatus {
    pub schema_name: String,
    pub applied_count: usize,
    pub latest_version: Option<String>,
    pub migrations: Vec<MigrationInfo>,
}

/// Migration info for status response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationInfo {
    pub version: String,
    pub name: String,
    pub applied_at: chrono::DateTime<chrono::Utc>,
    pub checksum: String,
}

impl From<AppliedMigration> for MigrationInfo {
    fn from(m: AppliedMigration) -> Self {
        Self {
            version: m.version,
            name: m.name,
            applied_at: m.applied_at,
            checksum: m.checksum,
        }
    }
}
