use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::{info, warn};

use super::ORMQuery;

/// Table-level permissions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TablePermission {
    pub table_name: String,
    pub readable_columns: HashSet<String>,
    pub allow_aggregations: bool,
    pub max_rows: Option<usize>,
}

/// ORM Permissions manager
pub struct ORMPermissions {
    // Challenge ID -> Table Name -> Permissions
    permissions: HashMap<String, HashMap<String, TablePermission>>,
}

impl ORMPermissions {
    pub fn new() -> Self {
        Self {
            permissions: HashMap::new(),
        }
    }

    /// Load permissions for a challenge
    pub fn load_permissions(
        &mut self,
        challenge_id: &str,
        permissions: HashMap<String, TablePermission>,
    ) -> Result<()> {
        info!(
            challenge_id = challenge_id,
            table_count = permissions.len(),
            "Loading permissions"
        );

        self.permissions
            .insert(challenge_id.to_string(), permissions);
        Ok(())
    }

    /// Check if a query is allowed
    pub fn check_query_permissions(&self, query: &ORMQuery) -> Result<()> {
        // Get challenge ID from schema name (format: challenge_<id> or <challenge_name>_v<version>)
        let challenge_id = query
            .schema
            .as_ref()
            .and_then(|s| {
                // Try challenge_<id> format first (UUID format)
                if let Some(id) = s.strip_prefix("challenge_") {
                    // Check if it looks like a UUID (contains hyphens) or just an ID
                    if id.contains('-') {
                        Some(id.to_string())
                    } else {
                        // Try to extract from <name>_v<version> format
                        s.rsplit_once("_v")
                            .map(|(name, _)| {
                                // Normalize challenge name (remove challenge_ prefix if present)
                                name.strip_prefix("challenge_").unwrap_or(name).to_string()
                            })
                            .or_else(|| Some(id.to_string()))
                    }
                } else {
                    // Extract challenge name from <name>_v<version> format
                    s.rsplit_once("_v").map(|(name, _)| name.to_string())
                }
            })
            .ok_or_else(|| anyhow::anyhow!("Invalid schema name"))?;

        // Get challenge permissions
        let challenge_perms = self.permissions.get(&challenge_id);

        // If no permissions are defined for this challenge, allow all (development mode)
        // This makes permissions optional - they provide additional security when defined
        if challenge_perms.is_none() {
            warn!(
                challenge_id = &challenge_id,
                table = &query.table,
                schema = query.schema.as_ref().unwrap_or(&"unknown".to_string()),
                "No permissions defined for challenge - allowing all operations (development mode)"
            );
            return Ok(()); // Allow all tables and columns
        }

        let challenge_perms = challenge_perms.unwrap();

        // Get table permissions
        let table_perms = challenge_perms
            .get(&query.table)
            .ok_or_else(|| anyhow::anyhow!("Access denied to table: {}", query.table))?;

        // Check column permissions
        if let Some(columns) = &query.columns {
            for column in columns {
                if !table_perms.readable_columns.contains(column) {
                    return Err(anyhow::anyhow!("Access denied to column: {}", column));
                }
            }
        }

        // Check filter column permissions
        if let Some(filters) = &query.filters {
            for filter in filters {
                if !table_perms.readable_columns.contains(&filter.column) {
                    return Err(anyhow::anyhow!(
                        "Access denied to filter column: {}",
                        filter.column
                    ));
                }
            }
        }

        // Check order by column permissions
        if let Some(order_by) = &query.order_by {
            for order in order_by {
                if !table_perms.readable_columns.contains(&order.column) {
                    return Err(anyhow::anyhow!(
                        "Access denied to order column: {}",
                        order.column
                    ));
                }
            }
        }

        // Check aggregation permissions
        if let Some(aggregations) = &query.aggregations {
            if !table_perms.allow_aggregations {
                return Err(anyhow::anyhow!(
                    "Aggregations not allowed for table: {}",
                    query.table
                ));
            }

            for agg in aggregations {
                if !table_perms.readable_columns.contains(&agg.column) {
                    return Err(anyhow::anyhow!(
                        "Access denied to aggregate column: {}",
                        agg.column
                    ));
                }
            }
        }

        // Check row limit
        if let Some(max_rows) = table_perms.max_rows {
            if let Some(limit) = query.limit {
                if limit > max_rows {
                    warn!(
                        table = &query.table,
                        requested = limit,
                        max = max_rows,
                        "Query limit exceeds maximum allowed"
                    );
                    return Err(anyhow::anyhow!(
                        "Query limit {} exceeds maximum allowed: {}",
                        limit,
                        max_rows
                    ));
                }
            }
        }

        Ok(())
    }

    /// Check if a challenge can access a table
    pub fn can_access_table(&self, challenge_id: &str, table_name: &str) -> bool {
        self.permissions
            .get(challenge_id)
            .and_then(|perms| perms.get(table_name))
            .is_some()
    }

    /// Get available tables for a challenge
    pub fn get_available_tables(&self, challenge_id: &str) -> Vec<String> {
        self.permissions
            .get(challenge_id)
            .map(|perms| perms.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Get readable columns for a table
    pub fn get_readable_columns(
        &self,
        challenge_id: &str,
        table_name: &str,
    ) -> Option<HashSet<String>> {
        self.permissions
            .get(challenge_id)
            .and_then(|perms| perms.get(table_name))
            .map(|table_perm| table_perm.readable_columns.clone())
    }
}

/// Default permissions for common tables
impl ORMPermissions {
    pub fn default_challenge_permissions() -> HashMap<String, TablePermission> {
        let mut permissions = HashMap::new();

        // Challenge submissions table
        permissions.insert(
            "challenge_submissions".to_string(),
            TablePermission {
                table_name: "challenge_submissions".to_string(),
                readable_columns: HashSet::from([
                    "id".to_string(),
                    "validator_hotkey".to_string(),
                    "miner_hotkey".to_string(),
                    "block_height".to_string(),
                    "score".to_string(),
                    "weight".to_string(),
                    "status".to_string(),
                    "created_at".to_string(),
                    "completed_at".to_string(),
                ]),
                allow_aggregations: true,
                max_rows: Some(10000),
            },
        );

        // Miner performance table
        permissions.insert(
            "miner_performance".to_string(),
            TablePermission {
                table_name: "miner_performance".to_string(),
                readable_columns: HashSet::from([
                    "id".to_string(),
                    "miner_hotkey".to_string(),
                    "epoch".to_string(),
                    "total_submissions".to_string(),
                    "successful_submissions".to_string(),
                    "failed_submissions".to_string(),
                    "average_score".to_string(),
                    "total_weight".to_string(),
                    "created_at".to_string(),
                ]),
                allow_aggregations: true,
                max_rows: Some(5000),
            },
        );

        // Weight recommendations table
        permissions.insert(
            "weight_recommendations".to_string(),
            TablePermission {
                table_name: "weight_recommendations".to_string(),
                readable_columns: HashSet::from([
                    "id".to_string(),
                    "epoch".to_string(),
                    "block_height".to_string(),
                    "total_miners".to_string(),
                    "active_miners".to_string(),
                    "submitted".to_string(),
                    "created_at".to_string(),
                ]),
                allow_aggregations: false,
                max_rows: Some(1000),
            },
        );

        permissions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permissions_check() {
        let mut permissions = ORMPermissions::new();
        let default_perms = ORMPermissions::default_challenge_permissions();
        permissions
            .load_permissions("test-challenge", default_perms)
            .unwrap();

        // Test valid query
        let query = ORMQuery {
            operation: "select".to_string(),
            table: "challenge_submissions".to_string(),
            schema: Some("challenge_test-challenge".to_string()),
            columns: Some(vec!["id".to_string(), "score".to_string()]),
            filters: None,
            order_by: None,
            limit: Some(100),
            offset: None,
            aggregations: None,
            values: None,
            set_values: None,
        };

        assert!(permissions.check_query_permissions(&query).is_ok());

        // Test invalid column
        let mut invalid_query = query.clone();
        invalid_query.columns = Some(vec!["id".to_string(), "secret_column".to_string()]);
        assert!(permissions.check_query_permissions(&invalid_query).is_err());
    }
}
