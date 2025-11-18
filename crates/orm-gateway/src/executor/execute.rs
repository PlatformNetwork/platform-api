//! Main query execution dispatcher

use anyhow::Result;
use std::time::Instant;

use crate::{ORMQuery, QueryResult};

use super::QueryExecutor;

impl QueryExecutor {
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
}
