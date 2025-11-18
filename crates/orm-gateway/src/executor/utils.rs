//! Utility functions for query building and execution

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as base64_engine, Engine as _};
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{Column, Row, TypeInfo};
use std::str::FromStr;

use crate::QueryFilter;

use super::QueryExecutor;

impl QueryExecutor {
    /// Build WHERE clause from filters
    pub(super) fn build_where_clause(
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
    pub(super) async fn execute_raw_query(
        &self,
        sql: &str,
        bind_values: Vec<serde_json::Value>,
    ) -> Result<Vec<serde_json::Value>> {
        let mut query = sqlx::query(sql);

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
                    let trimmed = s.trim();
                    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
                    {
                        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&s) {
                            query.bind::<serde_json::Value>(json_value)
                        } else {
                            query = self.bind_timestamp_or_string(query, &s);
                            query
                        }
                    } else {
                        query = self.bind_timestamp_or_string(query, &s);
                        query
                    }
                }
                serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                    query.bind::<serde_json::Value>(value)
                }
            };
        }

        let query = query.persistent(false);

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
                let mut error_msg = format!("Query execution failed for SQL: {}", sql);
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

        let mut json_rows = Vec::new();
        for row in rows {
            let mut json_row = serde_json::Map::new();

            for (i, column) in row.columns().iter().enumerate() {
                let column_name = column.name();
                let type_name = column.type_info().name();
                let value = self.convert_column_value(&row, i, type_name);
                json_row.insert(column_name.to_string(), value);
            }

            json_rows.push(serde_json::Value::Object(json_row));
        }

        Ok(json_rows)
    }

    /// Bind timestamp or string to query
    fn bind_timestamp_or_string<'a>(
        &self,
        mut query: sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments>,
        s: &str,
    ) -> sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments> {
        if s.len() >= 19 && (s.contains('T') || s.contains(' ')) {
            if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
                query.bind(ndt)
            } else if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
                query.bind(ndt)
            } else if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
                query.bind(ndt)
            } else if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
                query.bind(ndt)
            } else if s.ends_with('Z') || s.contains('+') || (s.matches('-').count() >= 4 && s.contains('T')) {
                if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                    query.bind(dt.with_timezone(&Utc))
                } else if let Ok(dt) = DateTime::<Utc>::from_str(s) {
                    query.bind(dt)
                } else {
                    query.bind(s.to_string())
                }
            } else {
                query.bind(s.to_string())
            }
        } else {
            query.bind(s.to_string())
        }
    }

    /// Convert PostgreSQL column value to JSON
    fn convert_column_value(
        &self,
        row: &sqlx::postgres::PgRow,
        i: usize,
        type_name: &str,
    ) -> serde_json::Value {
        match type_name {
            "INT2" => row.try_get::<i16, _>(i).ok().map(|v| serde_json::Value::Number((v as i64).into())).unwrap_or(serde_json::Value::Null),
            "INT4" => row.try_get::<i32, _>(i).ok().map(|v| serde_json::Value::Number((v as i64).into())).unwrap_or(serde_json::Value::Null),
            "INT8" => row.try_get::<i64, _>(i).ok().map(|v| serde_json::Value::Number(v.into())).unwrap_or(serde_json::Value::Null),
            "FLOAT4" => row.try_get::<f32, _>(i).ok().and_then(|v| serde_json::Number::from_f64(v as f64).map(serde_json::Value::Number)).unwrap_or(serde_json::Value::Null),
            "FLOAT8" => row.try_get::<f64, _>(i).ok().and_then(|v| serde_json::Number::from_f64(v).map(serde_json::Value::Number)).unwrap_or(serde_json::Value::Null),
            "NUMERIC" => {
                if let Ok(v) = row.try_get::<f64, _>(i) {
                    serde_json::Number::from_f64(v).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null)
                } else if let Ok(v) = row.try_get::<i64, _>(i) {
                    serde_json::Value::Number(v.into())
                } else if let Ok(v) = row.try_get::<i32, _>(i) {
                    serde_json::Value::Number((v as i64).into())
                } else {
                    serde_json::Value::Null
                }
            }
            "BOOL" => row.try_get::<bool, _>(i).ok().map(serde_json::Value::Bool).unwrap_or(serde_json::Value::Null),
            "TEXT" | "VARCHAR" | "CHAR" | "BPCHAR" | "NAME" => row.try_get::<String, _>(i).ok().map(serde_json::Value::String).unwrap_or(serde_json::Value::Null),
            "TIMESTAMP" => row.try_get::<NaiveDateTime, _>(i).ok().map(|v| serde_json::Value::String(v.format("%Y-%m-%dT%H:%M:%S%.f").to_string())).unwrap_or(serde_json::Value::Null),
            "TIMESTAMPTZ" => row.try_get::<DateTime<Utc>, _>(i).ok().map(|v| serde_json::Value::String(v.to_rfc3339())).unwrap_or(serde_json::Value::Null),
            "DATE" => row.try_get::<chrono::NaiveDate, _>(i).ok().map(|v| serde_json::Value::String(v.format("%Y-%m-%d").to_string())).unwrap_or(serde_json::Value::Null),
            "TIME" => row.try_get::<chrono::NaiveTime, _>(i).ok().map(|v| serde_json::Value::String(v.format("%H:%M:%S%.f").to_string())).unwrap_or(serde_json::Value::Null),
            "JSON" | "JSONB" => row.try_get::<serde_json::Value, _>(i).ok().unwrap_or(serde_json::Value::Null),
            "UUID" => row.try_get::<uuid::Uuid, _>(i).ok().map(|v| serde_json::Value::String(v.to_string())).unwrap_or(serde_json::Value::Null),
            "BYTEA" => row.try_get::<Vec<u8>, _>(i).ok().map(|v| serde_json::Value::String(base64_engine.encode(&v))).unwrap_or(serde_json::Value::Null),
            _ => {
                if let Ok(v) = row.try_get::<bool, _>(i) {
                    serde_json::Value::Bool(v)
                } else if let Ok(v) = row.try_get::<i64, _>(i) {
                    serde_json::Value::Number(v.into())
                } else if let Ok(v) = row.try_get::<i32, _>(i) {
                    serde_json::Value::Number((v as i64).into())
                } else if let Ok(v) = row.try_get::<f64, _>(i) {
                    serde_json::Number::from_f64(v).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null)
                } else if let Ok(v) = row.try_get::<String, _>(i) {
                    serde_json::Value::String(v)
                } else if let Ok(v) = row.try_get::<serde_json::Value, _>(i) {
                    v
                } else {
                    tracing::warn!("Unhandled PostgreSQL type '{}', returning NULL", type_name);
                    serde_json::Value::Null
                }
            }
        }
    }
}
