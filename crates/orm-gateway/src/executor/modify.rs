//! INSERT, UPDATE, DELETE query execution

use anyhow::Result;
use tracing::info;

use crate::{ORMQuery, QueryResult};

use super::QueryExecutor;

impl QueryExecutor {
    /// Execute INSERT query
    pub(super) async fn execute_insert(&self, query: &ORMQuery) -> Result<QueryResult> {
        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        sql.push_str("INSERT INTO ");
        if let Some(schema) = &query.schema {
            sql.push_str(&format!("{}.{}", schema, query.table));
        } else {
            sql.push_str(&query.table);
        }

        let values_data = query
            .values
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("INSERT requires values"))?;

        let columns: Vec<String> = values_data.iter().map(|cv| cv.column.clone()).collect();
        let values: Vec<&serde_json::Value> = values_data.iter().map(|cv| &cv.value).collect();

        sql.push_str(&format!(" ({}) VALUES (", columns.join(", ")));

        let placeholders: Vec<String> = (1..=values.len()).map(|i| format!("${}", i)).collect();
        sql.push_str(&placeholders.join(", "));
        sql.push(')');

        bind_values.extend(values.into_iter().cloned());

        info!(sql = &sql, "Executing INSERT query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: rows.len(),
            rows,
            execution_time_ms: 0,
        })
    }

    /// Execute UPDATE query
    pub(super) async fn execute_update(&self, query: &ORMQuery) -> Result<QueryResult> {
        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        sql.push_str("UPDATE ");
        if let Some(schema) = &query.schema {
            sql.push_str(&format!("{}.{}", schema, query.table));
        } else {
            sql.push_str(&query.table);
        }

        let set_values = query
            .set_values
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("UPDATE requires set_values"))?;

        sql.push_str(" SET ");
        let mut set_parts: Vec<String> = Vec::new();
        for cv in set_values.iter() {
            bind_values.push(cv.value.clone());
            set_parts.push(format!("{} = ${}", cv.column, bind_values.len()));
        }
        sql.push_str(&set_parts.join(", "));

        if let Some(filters) = &query.filters {
            if !filters.is_empty() {
                sql.push_str(" WHERE ");
                let where_parts = self.build_where_clause(filters, &mut bind_values)?;
                sql.push_str(&where_parts.join(" AND "));
            }
        }

        info!(sql = &sql, "Executing UPDATE query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: rows.len(),
            rows,
            execution_time_ms: 0,
        })
    }

    /// Execute DELETE query
    pub(super) async fn execute_delete(&self, query: &ORMQuery) -> Result<QueryResult> {
        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        sql.push_str("DELETE FROM ");
        if let Some(schema) = &query.schema {
            sql.push_str(&format!("{}.{}", schema, query.table));
        } else {
            sql.push_str(&query.table);
        }

        if let Some(filters) = &query.filters {
            if !filters.is_empty() {
                sql.push_str(" WHERE ");
                let where_parts = self.build_where_clause(filters, &mut bind_values)?;
                sql.push_str(&where_parts.join(" AND "));
            }
        }

        info!(sql = &sql, "Executing DELETE query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: rows.len(),
            rows,
            execution_time_ms: 0,
        })
    }
}
