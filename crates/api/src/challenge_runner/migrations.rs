use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::process::Command;
use std::io::Write;
use tempfile::NamedTempFile;
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

    /// Apply a single migration using psql for direct SQL execution
    /// This uses PostgreSQL's native parser, guaranteeing 100% compatibility with all SQL syntax
    async fn apply_single_migration(&self, schema_name: &str, migration: &Migration) -> Result<()> {
        info!(
            schema = schema_name,
            version = &migration.version,
            name = &migration.name,
            sql_length = migration.sql.len(),
            "Starting migration application via psql"
        );

        // Execute migration via psql (wrapped in transaction)
        self.execute_migration_via_psql(schema_name, migration).await?;

        // Record migration in schema_migrations table
        let record_query = format!(
            r#"
            INSERT INTO {}.schema_migrations (version, name, checksum)
            VALUES ($1, $2, $3)
            ON CONFLICT (version) DO NOTHING
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
            .execute(&self.pool)
            .await
        {
            Ok(result) => {
                if result.rows_affected() > 0 {
                    info!(
                        schema = schema_name,
                        version = &migration.version,
                        "✅ Migration recorded in schema_migrations"
                    );
                } else {
                    warn!(
                        schema = schema_name,
                        version = &migration.version,
                        "Migration already recorded in schema_migrations (skipped)"
                    );
                }
            }
            Err(e) => {
                error!(
                    schema = schema_name,
                    version = &migration.version,
                    error = %e,
                    "Failed to record migration in schema_migrations"
                );
                return Err(anyhow::anyhow!(
                    "Failed to record migration {} - {}: {}",
                    migration.version,
                    migration.name,
                    e
                ));
            }
        }

        info!(
            schema = schema_name,
            version = &migration.version,
            "✅ Migration applied successfully"
        );

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

    /// Execute migration via psql subprocess (most reliable method)
    /// This uses PostgreSQL's native parser, guaranteeing 100% compatibility with all SQL syntax
    /// The migration is wrapped in a transaction (BEGIN/COMMIT) for atomicity
    async fn execute_migration_via_psql(
        &self,
        schema_name: &str,
        migration: &Migration,
    ) -> Result<()> {
        info!(
            schema = schema_name,
            version = &migration.version,
            "Executing migration via psql subprocess"
        );

        // Get DATABASE_URL
        let database_url = std::env::var("DATABASE_URL")
            .context("DATABASE_URL environment variable not set")?;

        // Check if psql is available
        let psql_check = Command::new("which")
            .arg("psql")
            .output();
        
        match psql_check {
            Ok(output) if output.status.success() => {
                let psql_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                info!(
                    schema = schema_name,
                    version = &migration.version,
                    psql_path = %psql_path,
                    "psql found at: {}",
                    psql_path
                );
            }
            _ => {
                warn!(
                    schema = schema_name,
                    version = &migration.version,
                    "psql not found in PATH - migration may fail"
                );
            }
        }

        // Create temporary SQL file with migration content
        // Wrap in transaction and set search_path
        let full_sql = format!(
            "-- Migration: {} ({})\n\
             -- Schema: {}\n\
             BEGIN;\n\
             SET search_path TO {}, public;\n\n\
             {}\n\n\
             COMMIT;",
            migration.name,
            migration.version,
            schema_name,
            schema_name,
            migration.sql
        );
        
        let mut temp_file = NamedTempFile::new()
            .context("Failed to create temporary SQL file")?;
        
        temp_file
            .write_all(full_sql.as_bytes())
            .context("Failed to write migration SQL to temporary file")?;
        
        // Flush to ensure data is written
        temp_file.flush()
            .context("Failed to flush temporary SQL file")?;
        
        let temp_path = temp_file.path().to_str()
            .ok_or_else(|| anyhow::anyhow!("Failed to get temporary file path"))?;

        info!(
            schema = schema_name,
            version = &migration.version,
            temp_file = %temp_path,
            "Executing migration via psql from temporary file"
        );

        // Execute via psql
        // Use -v ON_ERROR_STOP=1 to stop on first error
        // Use -q for quiet mode (less verbose output)
        // Note: psql accepts PostgreSQL connection URLs directly as the first argument
        let output = Command::new("psql")
            .arg(&database_url)
            .arg("-f")
            .arg(temp_path)
            .arg("-v")
            .arg("ON_ERROR_STOP=1")
            .arg("-q") // Quiet mode
            .output()
            .with_context(|| {
                format!(
                    "Failed to execute psql command. DATABASE_URL: {}, temp_file: {}",
                    database_url.split('@').next().unwrap_or("(hidden)"),
                    temp_path
                )
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let exit_code = output.status.code().unwrap_or(-1);
            
            // Log full error details
            error!(
                schema = schema_name,
                version = &migration.version,
                exit_code = exit_code,
                stderr_length = stderr.len(),
                stdout_length = stdout.len(),
                "psql execution failed with exit code {}",
                exit_code
            );
            
            // Log stderr and stdout if they contain useful information
            if !stderr.is_empty() {
                error!(
                    schema = schema_name,
                    version = &migration.version,
                    stderr = %stderr,
                    "psql stderr output"
                );
            }
            if !stdout.is_empty() {
                warn!(
                    schema = schema_name,
                    version = &migration.version,
                    stdout = %stdout,
                    "psql stdout output"
                );
            }
            
            // Try to extract line number from error message
            let line_number = Self::extract_line_number_from_error(&stderr);
            let error_context = if let Some(line) = line_number {
                format!("Error at line {}", line)
            } else {
                format!("psql exited with code {}", exit_code)
            };
            
            // Provide SQL preview for context
            let sql_preview = migration.sql
                .lines()
                .take(10)
                .collect::<Vec<_>>()
                .join("\n");
            
            // Build comprehensive error message
            let mut error_msg = format!(
                "Migration {} ({}) failed via psql:\n{}\n\nExit code: {}\n",
                migration.version,
                migration.name,
                error_context,
                exit_code
            );
            
            if !stderr.is_empty() {
                error_msg.push_str(&format!("STDERR:\n{}\n\n", stderr));
            }
            if !stdout.is_empty() {
                error_msg.push_str(&format!("STDOUT:\n{}\n\n", stdout));
            }
            error_msg.push_str(&format!("SQL preview (first 10 lines):\n{}", sql_preview));
            
            return Err(anyhow::anyhow!(error_msg));
        }

        info!(
            schema = schema_name,
            version = &migration.version,
            "✅ Migration executed successfully via psql"
        );

        Ok(())
    }
    
    /// Extract line number from psql error message
    /// PostgreSQL errors often include "LINE X:" in the error message
    fn extract_line_number_from_error(error_msg: &str) -> Option<usize> {
        // Look for patterns like "LINE 5:" or "line 10:" or "at line 15"
        for line in error_msg.lines() {
            // Try "LINE X:" pattern
            if let Some(pos) = line.find("LINE ") {
                let rest = &line[pos + 5..];
                if let Some(end) = rest.find(':') {
                    if let Ok(num) = rest[..end].trim().parse::<usize>() {
                        return Some(num);
                    }
                }
            }
            // Try "line X:" pattern (lowercase)
            if let Some(pos) = line.find("line ") {
                let rest = &line[pos + 5..];
                if let Some(end) = rest.find(':') {
                    if let Ok(num) = rest[..end].trim().parse::<usize>() {
                        return Some(num);
                    }
                }
            }
            // Try "at line X" pattern
            if let Some(pos) = line.find("at line ") {
                let rest = &line[pos + 8..];
                if let Some(end) = rest.find(|c: char| c.is_whitespace() || c == ':') {
                    if let Ok(num) = rest[..end].trim().parse::<usize>() {
                        return Some(num);
                    }
                }
            }
        }
        None
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
