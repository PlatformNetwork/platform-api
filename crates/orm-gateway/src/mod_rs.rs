use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use tracing::info;

use crate::{executor::QueryExecutor, permissions::{ORMPermissions, TablePermission}, query_validator::QueryValidator};

/// Configuration for ORM Gateway
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ORMGatewayConfig {
    pub max_query_limit: usize,
    pub query_timeout: u64,
    pub allowed_operations: Vec<String>,
    pub enable_aggregations: bool,
    pub read_only: bool, // If true, only SELECT/COUNT allowed
}

impl Default for ORMGatewayConfig {
    fn default() -> Self {
        Self {
            max_query_limit: 1000,
            query_timeout: 30,
            allowed_operations: vec!["select".to_string(), "count".to_string()],
            enable_aggregations: true,
            read_only: true, // Default to read-only
        }
    }
}

impl ORMGatewayConfig {
    /// Create a read-write configuration (for public routes from SDK)
    pub fn read_write() -> Self {
        Self {
            max_query_limit: 1000,
            query_timeout: 30,
            allowed_operations: vec![
                "select".to_string(),
                "count".to_string(),
                "insert".to_string(),
                "update".to_string(),
                "delete".to_string(),
            ],
            enable_aggregations: true,
            read_only: false,
        }
    }

    /// Create a read-only configuration (for validator routes)
    pub fn read_only() -> Self {
        Self::default()
    }
}

/// Secure ORM Gateway for read-only queries
pub struct SecureORMGateway {
    config: ORMGatewayConfig,
    db_pool: PgPool,
    permissions: ORMPermissions,
    query_validator: QueryValidator,
    query_executor: QueryExecutor,
}

impl SecureORMGateway {
    pub fn new(config: ORMGatewayConfig, db_pool: PgPool) -> Self {
        let permissions = ORMPermissions::new();
        let query_validator = QueryValidator::new(config.clone());
        let query_executor = QueryExecutor::new(db_pool.clone(), config.query_timeout);

        Self {
            config,
            db_pool,
            permissions,
            query_validator,
            query_executor,
        }
    }

    /// Execute a query (read or write depending on config)
    pub async fn execute_query(&self, query: ORMQuery) -> Result<QueryResult> {
        // Check if write operations are allowed
        if self.config.read_only {
            match query.operation.as_str() {
                "select" | "count" => {}
                _ => {
                    return Err(anyhow::anyhow!(
                        "Write operations not allowed in read-only mode"
                    ));
                }
            }
        }

        // Validate query
        self.query_validator.validate(&query)?;

        // Check permissions
        self.permissions.check_query_permissions(&query)?;

        // Execute query
        let result = self.query_executor.execute(&query).await?;

        Ok(result)
    }

    /// Execute a read-only query (alias for compatibility)
    pub async fn execute_read_query(&self, query: ORMQuery) -> Result<QueryResult> {
        self.execute_query(query).await
    }

    /// Load permissions from a challenge
    pub async fn load_challenge_permissions(
        &mut self,
        challenge_id: &str,
        permissions: HashMap<String, TablePermission>,
    ) -> Result<()> {
        info!(
            challenge_id = challenge_id,
            table_count = permissions.len(),
            "Loading challenge permissions"
        );

        self.permissions
            .load_permissions(challenge_id, permissions)?;
        Ok(())
    }

    /// Get available tables for a challenge
    pub async fn get_available_tables(&self, challenge_id: &str) -> Vec<String> {
        self.permissions.get_available_tables(challenge_id)
    }

    /// Get table schema information
    pub async fn get_table_schema(
        &self,
        challenge_id: &str,
        table_name: &str,
    ) -> Result<TableSchema> {
        // Check if challenge has access to table
        if !self.permissions.can_access_table(challenge_id, table_name) {
            return Err(anyhow::anyhow!("Access denied to table: {}", table_name));
        }

        // Get schema from database
        let columns = sqlx::query(
            r#"
            SELECT 
                column_name,
                data_type,
                is_nullable,
                column_default
            FROM information_schema.columns
            WHERE table_schema = $1 AND table_name = $2
            ORDER BY ordinal_position
            "#,
        )
        .bind(format!("challenge_{}", challenge_id.replace('-', "_")))
        .bind(table_name)
        .fetch_all(&self.db_pool)
        .await?;

        let column_info: Vec<ColumnInfo> = columns
            .into_iter()
            .map(|col| {
                Ok(ColumnInfo {
                    name: col.try_get("column_name")?,
                    data_type: col.try_get("data_type")?,
                    nullable: col.try_get::<String, _>("is_nullable")? == "YES",
                    default: col.try_get("column_default").ok(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(TableSchema {
            table_name: table_name.to_string(),
            columns: column_info,
        })
    }
}

/// ORM Query structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ORMQuery {
    pub operation: String, // "select", "count", "insert", "update", "delete"
    pub table: String,
    pub schema: Option<String>,
    pub db_version: Option<u32>, // Database version for schema resolution
    pub columns: Option<Vec<String>>,
    pub filters: Option<Vec<QueryFilter>>,
    pub order_by: Option<Vec<OrderBy>>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub aggregations: Option<Vec<Aggregation>>,
    // For INSERT/UPDATE operations
    pub values: Option<Vec<ColumnValue>>, // For INSERT: column -> value mapping
    pub set_values: Option<Vec<ColumnValue>>, // For UPDATE: column -> value mapping
}

/// Column-value pair for INSERT/UPDATE operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnValue {
    pub column: String,
    pub value: serde_json::Value,
}

/// Query filter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryFilter {
    pub column: String,
    pub operator: String,
    pub value: serde_json::Value,
}

/// Order by clause
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBy {
    pub column: String,
    pub direction: String, // ASC or DESC
}

/// Aggregation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aggregation {
    pub function: String, // COUNT, SUM, AVG, MIN, MAX
    pub column: String,
    pub alias: String,
}

/// Query result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub rows: Vec<serde_json::Value>,
    pub row_count: usize,
    pub execution_time_ms: u64,
}

/// Table schema information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    pub table_name: String,
    pub columns: Vec<ColumnInfo>,
}

/// Column information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
}
