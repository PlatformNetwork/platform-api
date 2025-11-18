//! SELECT query execution

use anyhow::Result;
use tracing::info;

use crate::{ORMQuery, QueryResult};

use super::QueryExecutor;

impl QueryExecutor {
    /// Execute SELECT query
    pub(super) async fn execute_select(&self, query: &ORMQuery) -> Result<QueryResult> {
        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        sql.push_str("SELECT ");

        if let Some(aggregations) = &query.aggregations {
            let agg_parts: Vec<String> = aggregations
                .iter()
                .map(|agg| format!("{}({}) AS {}", agg.function, agg.column, agg.alias))
                .collect();
            sql.push_str(&agg_parts.join(", "));
        } else if let Some(columns) = &query.columns {
            sql.push_str(&columns.join(", "));
        } else {
            sql.push('*');
        }

        sql.push_str(" FROM ");
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

        if query.aggregations.is_some() {
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

        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }
        if let Some(offset) = query.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }

        info!(sql = &sql, "Executing SELECT query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: rows.len(),
            rows,
            execution_time_ms: 0,
        })
    }

    /// Execute COUNT query
    pub(super) async fn execute_count(&self, query: &ORMQuery) -> Result<QueryResult> {
        let mut sql = String::new();
        let mut bind_values: Vec<serde_json::Value> = Vec::new();

        sql.push_str("SELECT COUNT(*) as count FROM ");
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

        info!(sql = &sql, "Executing COUNT query");
        let rows = self.execute_raw_query(&sql, bind_values).await?;

        Ok(QueryResult {
            row_count: rows.len(),
            rows,
            execution_time_ms: 0,
        })
    }
}
