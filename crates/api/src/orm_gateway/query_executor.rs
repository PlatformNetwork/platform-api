use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use sqlx::{Column, PgPool, Row, TypeInfo};
use std::str::FromStr;
use std::time::Instant;
use tracing::info;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use super::{ColumnValue, ORMQuery, QueryFilter, QueryResult};

/// Query executor for safe SQL execution
pub struct QueryExecutor {
    db_pool: PgPool,
    query_timeout: u64,
}

impl QueryExecutor {
    pub fn new(db_pool: PgPool, query_timeout: u64) -> Self {
        Self {
            db_pool,
            query_timeout,
        }
    }

    /// Execute a validated query
    pub async fn execute(&self, query: &ORMQuery) -> Result<QueryResult> {
        let start_time = Instant::now();

        match query.operation.as_str() {
            "select" => self.execute_select(query).await,
            "count" => self.execute_count(query).await,
            "insert" => self.execute_insert(query).await,
            "update" => self.execute_update(query).await,
            "delete" => self.execute_delete(query).await,
            _ => Err(anyhow::anyhow!(
                "Unsupported operation: {}",
                query.operation
            )),
        }
        .map(|mut result| {
            result.execution_time_ms = start_time.elapsed().as_millis() as u64;
            result
        })
    }

    /// Execute SELECT query
    async fn execute_select(&self, query: &ORMQuery) -> Result<QueryResult> {
        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        // Build SELECT clause
        sql.push_str("SELECT ");

        if let Some(aggregations) = &query.aggregations {
            // Build aggregation select
            let agg_parts: Vec<String> = aggregations
                .iter()
                .map(|agg| format!("{}({}) AS {}", agg.function, agg.column, agg.alias))
                .collect();
            sql.push_str(&agg_parts.join(", "));
        } else if let Some(columns) = &query.columns {
            sql.push_str(&columns.join(", "));
        } else {
            sql.push_str("*");
        }

        // FROM clause
        sql.push_str(" FROM ");
        if let Some(schema) = &query.schema {
            sql.push_str(&format!("{}.{}", schema, query.table));
        } else {
            sql.push_str(&query.table);
        }

        // WHERE clause
        if let Some(filters) = &query.filters {
            if !filters.is_empty() {
                sql.push_str(" WHERE ");
                let where_parts = self.build_where_clause(filters, &mut bind_values)?;
                sql.push_str(&where_parts.join(" AND "));
            }
        }

        // GROUP BY clause (for aggregations)
        if query.aggregations.is_some() {
            // If we have aggregations, we need to group by non-aggregated columns
            if let Some(columns) = &query.columns {
                let non_agg_columns: Vec<&String> = columns
                    .iter()
                    .filter(|col| {
                        query
                            .aggregations
                            .as_ref()
                            .unwrap()
                            .iter()
                            .all(|agg| &agg.column != *col)
                    })
                    .collect();

                if !non_agg_columns.is_empty() {
                    sql.push_str(" GROUP BY ");
                    sql.push_str(
                        &non_agg_columns
                            .iter()
                            .map(|c| c.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
            }
        }

        // ORDER BY clause
        if let Some(order_by) = &query.order_by {
            if !order_by.is_empty() {
                sql.push_str(" ORDER BY ");
                let order_parts: Vec<String> = order_by
                    .iter()
                    .map(|o| format!("{} {}", o.column, o.direction))
                    .collect();
                sql.push_str(&order_parts.join(", "));
            }
        }

        // LIMIT and OFFSET
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }
        if let Some(offset) = query.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }

        // Execute query
        info!(sql = &sql, "Executing SELECT query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: rows.len(),
            rows,
            execution_time_ms: 0, // Will be set by caller
        })
    }

    /// Execute COUNT query
    async fn execute_count(&self, query: &ORMQuery) -> Result<QueryResult> {
        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        // Build COUNT query
        sql.push_str("SELECT COUNT(*) as count FROM ");

        if let Some(schema) = &query.schema {
            sql.push_str(&format!("{}.{}", schema, query.table));
        } else {
            sql.push_str(&query.table);
        }

        // WHERE clause
        if let Some(filters) = &query.filters {
            if !filters.is_empty() {
                sql.push_str(" WHERE ");
                let where_parts = self.build_where_clause(filters, &mut bind_values)?;
                sql.push_str(&where_parts.join(" AND "));
            }
        }

        // Execute query
        info!(sql = &sql, "Executing COUNT query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: 1,
            rows,
            execution_time_ms: 0, // Will be set by caller
        })
    }

    /// Execute INSERT query (all values are binded - no SQL injection possible)
    async fn execute_insert(&self, query: &ORMQuery) -> Result<QueryResult> {
        let values = query
            .values
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("INSERT operation requires 'values' field"))?;

        if values.is_empty() {
            return Err(anyhow::anyhow!(
                "INSERT operation requires at least one value"
            ));
        }

        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        // Build INSERT statement
        sql.push_str("INSERT INTO ");
        if let Some(schema) = &query.schema {
            sql.push_str(&format!("{}.{}", schema, query.table));
        } else {
            sql.push_str(&query.table);
        }

        // Build column list and placeholders
        let columns: Vec<String> = values.iter().map(|cv| cv.column.clone()).collect();
        // Simple placeholders - sqlx will handle JSONB typing automatically
        let placeholders: Vec<String> = (1..=values.len()).map(|i| format!("${}", i)).collect();

        sql.push_str(" (");
        sql.push_str(&columns.join(", "));
        sql.push_str(") VALUES (");
        sql.push_str(&placeholders.join(", "));
        sql.push_str(") RETURNING *");

        // Bind values
        for cv in values {
            bind_values.push(cv.value.clone());
        }

        // Execute query
        info!(sql = &sql, "Executing INSERT query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: rows.len(),
            rows,
            execution_time_ms: 0,
        })
    }

    /// Execute UPDATE query (all values are binded - no SQL injection possible)
    async fn execute_update(&self, query: &ORMQuery) -> Result<QueryResult> {
        let set_values = query
            .set_values
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("UPDATE operation requires 'set_values' field"))?;

        if set_values.is_empty() {
            return Err(anyhow::anyhow!(
                "UPDATE operation requires at least one set value"
            ));
        }

        if query.filters.is_none() || query.filters.as_ref().unwrap().is_empty() {
            return Err(anyhow::anyhow!(
                "UPDATE operation requires WHERE clause (filters)"
            ));
        }

        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        // Build UPDATE statement
        sql.push_str("UPDATE ");
        if let Some(schema) = &query.schema {
            sql.push_str(&format!("{}.{}", schema, query.table));
        } else {
            sql.push_str(&query.table);
        }

        // Build SET clause
        sql.push_str(" SET ");
        let set_parts: Vec<String> = set_values
            .iter()
            .enumerate()
            .map(|(i, cv)| {
                bind_values.push(cv.value.clone());
                // Simple placeholder - sqlx will handle JSONB typing automatically
                format!("{} = ${}", cv.column, bind_values.len())
            })
            .collect();
        sql.push_str(&set_parts.join(", "));

        // WHERE clause
        sql.push_str(" WHERE ");
        let where_parts =
            self.build_where_clause(query.filters.as_ref().unwrap(), &mut bind_values)?;
        sql.push_str(&where_parts.join(" AND "));

        sql.push_str(" RETURNING *");

        // Execute query
        info!(sql = &sql, "Executing UPDATE query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: rows.len(),
            rows,
            execution_time_ms: 0,
        })
    }

    /// Execute DELETE query (all values are binded - no SQL injection possible)
    async fn execute_delete(&self, query: &ORMQuery) -> Result<QueryResult> {
        if query.filters.is_none() || query.filters.as_ref().unwrap().is_empty() {
            return Err(anyhow::anyhow!(
                "DELETE operation requires WHERE clause (filters)"
            ));
        }

        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        // Build DELETE statement
        sql.push_str("DELETE FROM ");
        if let Some(schema) = &query.schema {
            sql.push_str(&format!("{}.{}", schema, query.table));
        } else {
            sql.push_str(&query.table);
        }

        // WHERE clause
        sql.push_str(" WHERE ");
        let where_parts =
            self.build_where_clause(query.filters.as_ref().unwrap(), &mut bind_values)?;
        sql.push_str(&where_parts.join(" AND "));

        sql.push_str(" RETURNING *");

        // Execute query
        info!(sql = &sql, "Executing DELETE query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: rows.len(),
            rows,
            execution_time_ms: 0,
        })
    }

    /// Build WHERE clause parts
    fn build_where_clause(
        &self,
        filters: &[QueryFilter],
        bind_values: &mut Vec<serde_json::Value>,
    ) -> Result<Vec<String>> {
        let mut parts = Vec::new();

        for filter in filters {
            let part = match filter.operator.to_uppercase().as_str() {
                "=" | "!=" | "<" | "<=" | ">" | ">=" => {
                    bind_values.push(filter.value.clone());
                    format!(
                        "{} {} ${}",
                        filter.column,
                        filter.operator,
                        bind_values.len()
                    )
                }
                "IN" | "NOT IN" => {
                    let array = filter
                        .value
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("IN operator requires array value"))?;

                    let placeholders: Vec<String> = array
                        .iter()
                        .map(|v| {
                            bind_values.push(v.clone());
                            format!("${}", bind_values.len())
                        })
                        .collect();

                    format!(
                        "{} {} ({})",
                        filter.column,
                        filter.operator,
                        placeholders.join(", ")
                    )
                }
                "LIKE" | "NOT LIKE" => {
                    bind_values.push(filter.value.clone());
                    format!(
                        "{} {} ${}",
                        filter.column,
                        filter.operator,
                        bind_values.len()
                    )
                }
                "IS NULL" => format!("{} IS NULL", filter.column),
                "IS NOT NULL" => format!("{} IS NOT NULL", filter.column),
                _ => return Err(anyhow::anyhow!("Unsupported operator: {}", filter.operator)),
            };

            parts.push(part);
        }

        Ok(parts)
    }

    /// Execute raw SQL query with bindings
    async fn execute_raw_query(
        &self,
        sql: &str,
        bind_values: Vec<serde_json::Value>,
    ) -> Result<Vec<serde_json::Value>> {
        // Create a dynamic query
        let mut query = sqlx::query(sql);

        // Bind values with automatic timestamp conversion
        for value in bind_values {
            query = match value {
                serde_json::Value::Null => query.bind(None::<String>),
                serde_json::Value::Bool(b) => query.bind(b),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        query.bind(i)
                    } else if let Some(f) = n.as_f64() {
                        query.bind(f)
                    } else {
                        return Err(anyhow::anyhow!("Unsupported number type"));
                    }
                }
                serde_json::Value::String(s) => {
                    // First check if it's a JSON string (starts with { or [)
                    // This handles cases where Python's json.dumps() sends JSON as a string
                    // but PostgreSQL expects JSONB
                    let trimmed = s.trim();
                    if (trimmed.starts_with('{') && trimmed.ends_with('}')) 
                        || (trimmed.starts_with('[') && trimmed.ends_with(']')) {
                        // Try to parse as JSON
                        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&s) {
                            // Successfully parsed as JSON, bind as JSONB
                            query.bind::<serde_json::Value>(json_value)
                        } else {
                            // Not valid JSON, continue with other checks
                            // Try to parse as ISO timestamp
                            if s.len() >= 19 && (s.contains('T') || s.contains(' ')) {
                                // First try NaiveDateTime (for timestamp without time zone columns)
                                if let Ok(ndt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S%.f") {
                                    query.bind(ndt)
                                } else if let Ok(ndt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f") {
                                    query.bind(ndt)
                                } else if let Ok(ndt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S") {
                                    query.bind(ndt)
                                } else if let Ok(ndt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S") {
                                    query.bind(ndt)
                                } else if s.ends_with('Z') || s.contains('+') || (s.matches('-').count() >= 4 && s.contains('T')) {
                                    // Has timezone indicator, try DateTime<Utc>
                                    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
                                        query.bind(dt.with_timezone(&Utc))
                                    } else if let Ok(dt) = DateTime::<Utc>::from_str(&s) {
                                        query.bind(dt)
                                    } else {
                                        query.bind(s)
                                    }
                                } else {
                                    query.bind(s)
                                }
                            } else {
                                query.bind(s)
                            }
                        }
                    } else {
                        // Doesn't look like JSON, try to parse as ISO timestamp
                        if s.len() >= 19 && (s.contains('T') || s.contains(' ')) {
                            // First try NaiveDateTime (for timestamp without time zone columns)
                            if let Ok(ndt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S%.f") {
                                query.bind(ndt)
                            } else if let Ok(ndt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f") {
                                query.bind(ndt)
                            } else if let Ok(ndt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S") {
                                query.bind(ndt)
                            } else if let Ok(ndt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S") {
                                query.bind(ndt)
                            } else if s.ends_with('Z') || s.contains('+') || (s.matches('-').count() >= 4 && s.contains('T')) {
                                // Has timezone indicator, try DateTime<Utc>
                                if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
                                    query.bind(dt.with_timezone(&Utc))
                                } else if let Ok(dt) = DateTime::<Utc>::from_str(&s) {
                                    query.bind(dt)
                                } else {
                                    query.bind(s)
                                }
                            } else {
                                query.bind(s)
                            }
                        } else {
                            // Doesn't look like a timestamp, bind as string
                            query.bind(s)
                        }
                    }
                }
                // Object and Array can be bound directly as JSONB
                serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                    query.bind::<serde_json::Value>(value)
                }
            };
        }

        // Set query timeout
        let query = query.persistent(false);

        // Execute with timeout
        let rows_result = tokio::time::timeout(
            std::time::Duration::from_secs(self.query_timeout),
            query.fetch_all(&self.db_pool),
        )
        .await
        .context(format!(
            "Query timeout after {} seconds",
            self.query_timeout
        ));

        let rows = match rows_result {
            Ok(Ok(rows)) => rows,
            Ok(Err(e)) => {
                // SQL execution error - extract detailed error information
                let mut error_msg = format!("Query execution failed for SQL: {}", sql);

                // Extract PostgreSQL error details if available
                if let Some(db_err) = e.as_database_error() {
                    error_msg.push_str(&format!("\nPostgreSQL Error Code: {:?}", db_err.code()));
                    error_msg
                        .push_str(&format!("\nPostgreSQL Error Message: {}", db_err.message()));
                    if let Some(constraint) = db_err.constraint() {
                        error_msg.push_str(&format!("\nConstraint: {}", constraint));
                    }
                    if let Some(table) = db_err.table() {
                        error_msg.push_str(&format!("\nTable: {}", table));
                    }
                } else {
                    // Fallback to general error message
                    error_msg.push_str(&format!("\nError: {}", e));
                }

                return Err(anyhow::anyhow!(error_msg));
            }
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Query timeout after {} seconds for SQL: {}",
                    self.query_timeout,
                    sql
                ));
            }
        };

        // Convert rows to JSON
        let mut json_rows = Vec::new();
        for row in rows {
            let mut json_row = serde_json::Map::new();

            // Get column names and values
            for (i, column) in row.columns().iter().enumerate() {
                let column_name = column.name();
                let type_name = column.type_info().name();
                
                let value: serde_json::Value = match type_name {
                    // Integer types
                    "INT2" => {
                        // SMALLINT
                        if let Ok(v) = row.try_get::<i16, _>(i) {
                            serde_json::Value::Number((v as i64).into())
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    "INT4" => {
                        // INTEGER
                        if let Ok(v) = row.try_get::<i32, _>(i) {
                            serde_json::Value::Number((v as i64).into())
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    "INT8" => {
                        // BIGINT
                        if let Ok(v) = row.try_get::<i64, _>(i) {
                            serde_json::Value::Number(v.into())
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    // Floating-point types
                    "FLOAT4" => {
                        // REAL
                        if let Ok(v) = row.try_get::<f32, _>(i) {
                            serde_json::Number::from_f64(v as f64)
                                .map(serde_json::Value::Number)
                                .unwrap_or(serde_json::Value::Null)
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    "FLOAT8" => {
                        // DOUBLE PRECISION
                        if let Ok(v) = row.try_get::<f64, _>(i) {
                            serde_json::Number::from_f64(v)
                                .map(serde_json::Value::Number)
                                .unwrap_or(serde_json::Value::Null)
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    "NUMERIC" => {
                        // NUMERIC/DECIMAL - try as f64 first, then i64
                        if let Ok(v) = row.try_get::<f64, _>(i) {
                            serde_json::Number::from_f64(v)
                                .map(serde_json::Value::Number)
                                .unwrap_or(serde_json::Value::Null)
                        } else if let Ok(v) = row.try_get::<i64, _>(i) {
                            serde_json::Value::Number(v.into())
                        } else if let Ok(v) = row.try_get::<i32, _>(i) {
                            serde_json::Value::Number((v as i64).into())
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    // Boolean type
                    "BOOL" => {
                        if let Ok(v) = row.try_get::<bool, _>(i) {
                            serde_json::Value::Bool(v)
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    // String types
                    "TEXT" | "VARCHAR" | "CHAR" | "BPCHAR" | "NAME" => {
                        if let Ok(v) = row.try_get::<String, _>(i) {
                            serde_json::Value::String(v)
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    // Timestamp types
                    "TIMESTAMP" => {
                        // TIMESTAMP without timezone
                        if let Ok(v) = row.try_get::<NaiveDateTime, _>(i) {
                            serde_json::Value::String(v.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    "TIMESTAMPTZ" => {
                        // TIMESTAMP with timezone
                        if let Ok(v) = row.try_get::<chrono::DateTime<chrono::Utc>, _>(i) {
                            serde_json::Value::String(v.to_rfc3339())
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    "DATE" => {
                        if let Ok(v) = row.try_get::<chrono::NaiveDate, _>(i) {
                            serde_json::Value::String(v.format("%Y-%m-%d").to_string())
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    "TIME" => {
                        if let Ok(v) = row.try_get::<chrono::NaiveTime, _>(i) {
                            serde_json::Value::String(v.format("%H:%M:%S%.f").to_string())
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    // JSON types
                    "JSON" | "JSONB" => {
                        if let Ok(v) = row.try_get::<serde_json::Value, _>(i) {
                            v
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    // UUID type
                    "UUID" => {
                        if let Ok(v) = row.try_get::<uuid::Uuid, _>(i) {
                            serde_json::Value::String(v.to_string())
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    // Bytea (binary data)
                    "BYTEA" => {
                        if let Ok(v) = row.try_get::<Vec<u8>, _>(i) {
                            // Encode as base64 for JSON transport
                            serde_json::Value::String(base64_engine.encode(&v))
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    // Fallback: try common types in order
                    _ => {
                        // Try to infer the type
                        if let Ok(v) = row.try_get::<bool, _>(i) {
                            serde_json::Value::Bool(v)
                        } else if let Ok(v) = row.try_get::<i64, _>(i) {
                            serde_json::Value::Number(v.into())
                        } else if let Ok(v) = row.try_get::<i32, _>(i) {
                            serde_json::Value::Number((v as i64).into())
                        } else if let Ok(v) = row.try_get::<f64, _>(i) {
                            serde_json::Number::from_f64(v)
                                .map(serde_json::Value::Number)
                                .unwrap_or(serde_json::Value::Null)
                        } else if let Ok(v) = row.try_get::<String, _>(i) {
                            serde_json::Value::String(v)
                        } else if let Ok(v) = row.try_get::<serde_json::Value, _>(i) {
                            v
                        } else {
                            // Last resort: NULL
                            tracing::warn!(
                                "Unhandled PostgreSQL type '{}' for column '{}', returning NULL",
                                type_name,
                                column_name
                            );
                            serde_json::Value::Null
                        }
                    }
                };

                json_row.insert(column_name.to_string(), value);
            }

            json_rows.push(serde_json::Value::Object(json_row));
        }

        Ok(json_rows)
    }
}
