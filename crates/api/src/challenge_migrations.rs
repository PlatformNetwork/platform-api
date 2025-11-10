use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::collections::HashMap;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationRequest {
    pub challenge_id: String,
    pub schema_name: String,
    pub migrations: Vec<Migration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Migration {
    pub version: String,
    pub name: String,
    pub sql: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStatus {
    pub version: String,
    pub name: String,
    pub applied_at: chrono::DateTime<chrono::Utc>,
    pub checksum: String,
}

pub struct MigrationOrchestrator {
    pool: PgPool,
}

impl MigrationOrchestrator {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a schema-specific database URL for a challenge
    pub async fn create_challenge_database(
        &self,
        challenge_id: &str,
        schema_name: &str,
    ) -> Result<String> {
        // Create schema if it doesn't exist
        sqlx::query(&format!(
            "CREATE SCHEMA IF NOT EXISTS {} AUTHORIZATION CURRENT_USER",
            schema_name
        ))
        .execute(&self.pool)
        .await
        .context("Failed to create schema")?;

        // Grant permissions to the schema
        sqlx::query(&format!(
            "GRANT ALL ON SCHEMA {} TO CURRENT_USER",
            schema_name
        ))
        .execute(&self.pool)
        .await
        .context("Failed to grant schema permissions")?;

        // Create migration tracking table in the schema
        sqlx::query(&format!(
            r#"
            CREATE TABLE IF NOT EXISTS {}.schema_migrations (
                version VARCHAR(255) PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                checksum VARCHAR(64) NOT NULL
            )
            "#,
            schema_name
        ))
        .execute(&self.pool)
        .await
        .context("Failed to create migrations table")?;

        info!(
            challenge_id = challenge_id,
            schema = schema_name,
            "Created challenge database schema"
        );

        Ok(schema_name.to_string())
    }

    /// Apply migrations to a challenge's schema
    pub async fn apply_migrations(
        &self,
        request: MigrationRequest,
    ) -> Result<Vec<MigrationStatus>> {
        let schema_name = &request.schema_name;

        // Get applied migrations
        let applied_migrations = self.get_applied_migrations(schema_name).await?;
        let applied_map: HashMap<String, MigrationStatus> = applied_migrations
            .into_iter()
            .map(|m| (m.version.clone(), m))
            .collect();

        let mut results = Vec::new();

        for migration in request.migrations {
            if let Some(existing) = applied_map.get(&migration.version) {
                // Check if migration matches
                if existing.checksum != migration.checksum {
                    warn!(
                        version = &migration.version,
                        expected = &migration.checksum,
                        actual = &existing.checksum,
                        "Migration checksum mismatch"
                    );
                    return Err(anyhow::anyhow!(
                        "Migration {} has different checksum",
                        migration.version
                    ));
                }
                // Skip already applied migration
                continue;
            }

            // Apply the migration
            let status = self.apply_single_migration(schema_name, migration).await?;
            results.push(status);
        }

        Ok(results)
    }

    /// Apply a single migration
    async fn apply_single_migration(
        &self,
        schema_name: &str,
        migration: Migration,
    ) -> Result<MigrationStatus> {
        // Start transaction
        let mut tx = self.pool.begin().await?;

        // Set search path to the schema
        sqlx::query(&format!("SET search_path TO {}, public", schema_name))
            .execute(&mut *tx)
            .await?;

        // Execute migration SQL
        sqlx::query(&migration.sql)
            .execute(&mut *tx)
            .await
            .context(format!("Failed to apply migration {}", migration.version))?;

        // Record migration
        let applied_at = chrono::Utc::now();
        sqlx::query(&format!(
            r#"
            INSERT INTO {}.schema_migrations (version, name, checksum, applied_at)
            VALUES ($1, $2, $3, $4)
            "#,
            schema_name
        ))
        .bind(&migration.version)
        .bind(&migration.name)
        .bind(&migration.checksum)
        .bind(&applied_at)
        .execute(&mut *tx)
        .await?;

        // Commit transaction
        tx.commit().await?;

        info!(
            schema = schema_name,
            version = &migration.version,
            name = &migration.name,
            "Applied migration"
        );

        Ok(MigrationStatus {
            version: migration.version,
            name: migration.name,
            applied_at,
            checksum: migration.checksum,
        })
    }

    /// Get list of applied migrations for a schema
    async fn get_applied_migrations(&self, schema_name: &str) -> Result<Vec<MigrationStatus>> {
        let migrations =
            sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>, String)>(&format!(
                r#"
                SELECT version, name, applied_at, checksum
                FROM {}.schema_migrations
                ORDER BY version
                "#,
                schema_name
            ))
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

        Ok(migrations
            .into_iter()
            .map(|(version, name, applied_at, checksum)| MigrationStatus {
                version,
                name,
                applied_at,
                checksum,
            })
            .collect())
    }

    /// Generate database credentials for a challenge
    pub async fn generate_challenge_credentials(
        &self,
        challenge_id: &str,
        schema_name: &str,
    ) -> Result<HashMap<String, String>> {
        // Get database connection info
        let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL not set")?;

        // Parse the URL to extract components
        let parsed = url::Url::parse(&database_url)?;
        let host = parsed.host_str().unwrap_or("localhost");
        let port = parsed.port().unwrap_or(5432);
        let database = parsed.path().trim_start_matches('/');
        let username = parsed.username();
        let password = parsed.password().unwrap_or("");

        // Create schema-specific connection string
        let schema_url = format!(
            "postgresql://{}:{}@{}:{}/{}?options=--search_path%3D{}",
            username, password, host, port, database, schema_name
        );

        let mut credentials = HashMap::new();
        credentials.insert("database_url".to_string(), schema_url.clone());
        credentials.insert("schema_name".to_string(), schema_name.to_string());
        credentials.insert("challenge_id".to_string(), challenge_id.to_string());

        // For SQLAlchemy async
        let async_url = schema_url.replace("postgresql://", "postgresql+asyncpg://");
        credentials.insert("async_database_url".to_string(), async_url);

        Ok(credentials)
    }
}
