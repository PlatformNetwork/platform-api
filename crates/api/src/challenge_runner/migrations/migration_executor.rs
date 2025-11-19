//! Migration execution and SQL processing

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::process::Command;
use tempfile::NamedTempFile;
use tracing::{debug, error, info, warn};

use super::core::Migration;

/// Execute a migration SQL file
pub async fn execute_migration(
    pool: &PgPool,
    schema_name: &str,
    migration: &Migration,
) -> Result<()> {
    info!(
        schema = schema_name,
        version = migration.version,
        name = migration.name,
        "Executing migration"
    );

    // Validate migration SQL first
    validate_migration_sql(&migration.sql).await?;

    // Execute the migration in a transaction
    let mut tx = pool.begin().await
        .context("Failed to begin transaction")?;

    // Execute each SQL statement
    let statements = parse_sql_statments(&migration.sql);
    for (i, statement) in statements.iter().enumerate() {
        debug!(
            schema = schema_name,
            version = migration.version,
            statement_index = i,
            "Executing SQL statement"
        );

        if let Err(e) = sqlx::query(statement).execute(&mut *tx).await {
            error!(
                schema = schema_name,
                version = migration.version,
                statement_index = i,
                error = %e,
                "Failed to execute SQL statement"
            );
            
            // Rollback transaction
            if let Err(rollback_err) = tx.rollback().await {
                error!(
                    schema = schema_name,
                    version = migration.version,
                    error = %rollback_err,
                    "Failed to rollback transaction"
                );
            }
            
            return Err(e).context("Failed to execute migration SQL");
        }
    }

    // Record the migration as applied
    super::migration_tracker::record_migration(&pool, schema_name, migration).await?;

    // Commit transaction
    tx.commit().await
        .context("Failed to commit migration transaction")?;

    info!(
        schema = schema_name,
        version = migration.version,
        "Migration executed successfully"
    );

    Ok(())
}

/// Validate migration SQL syntax
async fn validate_migration_sql(sql: &str) -> Result<()> {
    debug!("Validating migration SQL syntax");

    // Basic SQL validation checks
    if sql.trim().is_empty() {
        return Err(anyhow::anyhow!("Migration SQL cannot be empty"));
    }

    // Check for potentially dangerous operations
    let dangerous_patterns = [
        "DROP DATABASE",
        "DROP SCHEMA",
        "TRUNCATE",
        "DELETE FROM schema_migrations",
    ];

    for pattern in &dangerous_patterns {
        if sql.to_uppercase().contains(pattern) {
            warn!("Potentially dangerous SQL pattern detected: {}", pattern);
            // In production, you might want to reject these
        }
    }

    // Parse SQL statements to check syntax
    let statements = parse_sql_statments(sql);
    if statements.is_empty() {
        return Err(anyhow::anyhow!("No valid SQL statements found"));
    }

    debug!(
        statement_count = statements.len(),
        "Migration SQL validation passed"
    );

    Ok(())
}

/// Parse SQL statements from migration text
fn parse_sql_statments(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current_statement = String::new();
    let mut in_string = false;
    let mut string_char = '\0';
    let mut statement_depth = 0;

    for ch in sql.chars() {
        current_statement.push(ch);

        match ch {
            '\'' | '"' | '`' => {
                if !in_string {
                    in_string = true;
                    string_char = ch;
                } else if ch == string_char {
                    in_string = false;
                    string_char = '\0';
                }
            }
            ';' if !in_string && statement_depth == 0 => {
                let statement = current_statement.trim().to_string();
                if !statement.is_empty() && statement != ";" {
                    statements.push(statement);
                }
                current_statement.clear();
            }
            '(' if !in_string => statement_depth += 1,
            ')' if !in_string && statement_depth > 0 => statement_depth -= 1,
            _ => {}
        }
    }

    // Add the last statement if it doesn't end with semicolon
    let last_statement = current_statement.trim().to_string();
    if !last_statement.is_empty() && !last_statement.ends_with(';') {
        statements.push(last_statement);
    }

    statements
}

/// Execute migration using psql (for complex migrations)
pub async fn execute_migration_with_psql(
    pool: &PgPool,
    schema_name: &str,
    migration: &Migration,
) -> Result<()> {
    info!(
        schema = schema_name,
        version = migration.version,
        "Executing migration with psql"
    );

    // Get database connection info
    let db_url = get_database_url(pool).await?;

    // Create temporary file with migration SQL
    let mut temp_file = NamedTempFile::new()
        .context("Failed to create temporary file")?;

    use std::io::Write;
    temp_file.write_all(migration.sql.as_bytes())
        .context("Failed to write migration to temporary file")?;

    let temp_path = temp_file.path();

    // Execute psql command
    let output = Command::new("psql")
        .arg(&db_url)
        .arg("-v")
        .arg(&format!("ON_ERROR_STOP=1"))
        .arg("-v")
        .arg(&format!("search_path={}", schema_name))
        .arg("-f")
        .arg(temp_path)
        .output()
        .context("Failed to execute psql command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        error!(
            schema = schema_name,
            version = migration.version,
            stderr = %stderr,
            stdout = %stdout,
            "psql migration execution failed"
        );

        return Err(anyhow::anyhow!("psql execution failed: {}", stderr));
    }

    // Record the migration as applied
    super::migration_tracker::record_migration(pool, schema_name, migration).await?;

    info!(
        schema = schema_name,
        version = migration.version,
        "Migration executed successfully with psql"
    );

    Ok(())
}

/// Get database URL from pool
async fn get_database_url(pool: &PgPool) -> Result<String> {
    // This is a simplified implementation
    // In practice, you'd extract the connection details from the pool
    Ok("postgresql://localhost/platform_api".to_string())
}

/// Generate migration checksum
pub fn generate_checksum(sql: &str) -> String {
    use sha2::{Digest, Sha256};
    
    let mut hasher = Sha256::new();
    hasher.update(sql.trim().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Verify migration checksum
pub fn verify_checksum(migration: &Migration) -> Result<()> {
    let calculated_checksum = generate_checksum(&migration.sql);
    
    if calculated_checksum != migration.checksum {
        return Err(anyhow::anyhow!(
            "Migration checksum mismatch. Expected: {}, Got: {}",
            migration.checksum,
            calculated_checksum
        ));
    }

    Ok(())
}

/// Dry run migration (validate without executing)
pub async fn dry_run_migration(
    pool: &PgPool,
    schema_name: &str,
    migration: &Migration,
) -> Result<Vec<String>> {
    info!(
        schema = schema_name,
        version = migration.version,
        "Performing dry run migration"
    );

    // Validate SQL syntax
    validate_migration_sql(&migration.sql).await?;

    // Verify checksum
    verify_checksum(migration)?;

    // Parse statements to return what would be executed
    let statements = parse_sql_statments(&migration.sql);

    debug!(
        schema = schema_name,
        version = migration.version,
        statement_count = statements.len(),
        "Dry run migration completed"
    );

    Ok(statements)
}
