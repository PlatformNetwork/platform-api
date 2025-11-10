use anyhow::Result;
use std::collections::HashSet;
use tracing::warn;

use super::{ORMGatewayConfig, ORMQuery};

/// Query validator to ensure queries are safe and allowed
pub struct QueryValidator {
    config: ORMGatewayConfig,
    allowed_operators: HashSet<String>,
    allowed_aggregations: HashSet<String>,
}

impl QueryValidator {
    pub fn new(config: ORMGatewayConfig) -> Self {
        let allowed_operators = HashSet::from([
            "=".to_string(),
            "!=".to_string(),
            "<".to_string(),
            "<=".to_string(),
            ">".to_string(),
            ">=".to_string(),
            "IN".to_string(),
            "NOT IN".to_string(),
            "LIKE".to_string(),
            "NOT LIKE".to_string(),
            "IS NULL".to_string(),
            "IS NOT NULL".to_string(),
        ]);

        let allowed_aggregations = HashSet::from([
            "COUNT".to_string(),
            "SUM".to_string(),
            "AVG".to_string(),
            "MIN".to_string(),
            "MAX".to_string(),
        ]);

        Self {
            config,
            allowed_operators,
            allowed_aggregations,
        }
    }

    /// Validate a query
    pub fn validate(&self, query: &ORMQuery) -> Result<()> {
        // Check operation is allowed
        if !self.config.allowed_operations.contains(&query.operation) {
            return Err(anyhow::anyhow!(
                "Operation not allowed: {}",
                query.operation
            ));
        }

        // Validate table name (prevent SQL injection)
        self.validate_identifier(&query.table, "table")?;

        // Validate schema if present
        if let Some(schema) = &query.schema {
            self.validate_identifier(schema, "schema")?;
        }

        // Validate columns
        if let Some(columns) = &query.columns {
            if columns.is_empty() {
                return Err(anyhow::anyhow!("Column list cannot be empty"));
            }

            for column in columns {
                self.validate_identifier(column, "column")?;
            }
        }

        // Validate filters
        if let Some(filters) = &query.filters {
            for filter in filters {
                self.validate_identifier(&filter.column, "filter column")?;

                if !self
                    .allowed_operators
                    .contains(&filter.operator.to_uppercase())
                {
                    return Err(anyhow::anyhow!(
                        "Filter operator not allowed: {}",
                        filter.operator
                    ));
                }

                // Validate filter value based on operator
                self.validate_filter_value(&filter.operator, &filter.value)?;
            }
        }

        // Validate order by
        if let Some(order_by) = &query.order_by {
            for order in order_by {
                self.validate_identifier(&order.column, "order column")?;

                let direction = order.direction.to_uppercase();
                if direction != "ASC" && direction != "DESC" {
                    return Err(anyhow::anyhow!(
                        "Invalid order direction: {}",
                        order.direction
                    ));
                }
            }
        }

        // Validate limit
        if let Some(limit) = query.limit {
            if limit == 0 {
                return Err(anyhow::anyhow!("Limit cannot be zero"));
            }

            if limit > self.config.max_query_limit {
                warn!(
                    requested = limit,
                    max = self.config.max_query_limit,
                    "Query limit exceeds maximum"
                );
                return Err(anyhow::anyhow!(
                    "Query limit {} exceeds maximum allowed: {}",
                    limit,
                    self.config.max_query_limit
                ));
            }
        }

        // Validate aggregations
        if let Some(aggregations) = &query.aggregations {
            if !self.config.enable_aggregations {
                return Err(anyhow::anyhow!("Aggregations are not enabled"));
            }

            for agg in aggregations {
                self.validate_identifier(&agg.column, "aggregation column")?;
                self.validate_identifier(&agg.alias, "aggregation alias")?;

                if !self
                    .allowed_aggregations
                    .contains(&agg.function.to_uppercase())
                {
                    return Err(anyhow::anyhow!(
                        "Aggregation function not allowed: {}",
                        agg.function
                    ));
                }
            }
        }

        // Validate INSERT/UPDATE/DELETE operations
        match query.operation.as_str() {
            "insert" => {
                let values = query
                    .values
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("INSERT requires 'values' field"))?;

                if values.is_empty() {
                    return Err(anyhow::anyhow!("INSERT requires at least one value"));
                }

                if values.len() > 100 {
                    return Err(anyhow::anyhow!("INSERT cannot have more than 100 columns"));
                }

                for cv in values {
                    self.validate_identifier(&cv.column, "INSERT column")?;
                    // Value is validated as JSON (already parsed)
                }
            }
            "update" => {
                let set_values = query
                    .set_values
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("UPDATE requires 'set_values' field"))?;

                if set_values.is_empty() {
                    return Err(anyhow::anyhow!("UPDATE requires at least one set value"));
                }

                if set_values.len() > 100 {
                    return Err(anyhow::anyhow!(
                        "UPDATE cannot update more than 100 columns"
                    ));
                }

                for cv in set_values {
                    self.validate_identifier(&cv.column, "UPDATE column")?;
                }

                if query.filters.is_none() || query.filters.as_ref().unwrap().is_empty() {
                    return Err(anyhow::anyhow!("UPDATE requires WHERE clause (filters)"));
                }
            }
            "delete" => {
                if query.filters.is_none() || query.filters.as_ref().unwrap().is_empty() {
                    return Err(anyhow::anyhow!("DELETE requires WHERE clause (filters)"));
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Validate identifier to prevent SQL injection
    fn validate_identifier(&self, identifier: &str, context: &str) -> Result<()> {
        // Check for empty
        if identifier.is_empty() {
            return Err(anyhow::anyhow!("{} cannot be empty", context));
        }

        // Check length
        if identifier.len() > 128 {
            return Err(anyhow::anyhow!("{} name too long: {}", context, identifier));
        }

        // Only allow alphanumeric, underscore, and dot (for schema.table)
        if !identifier
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return Err(anyhow::anyhow!(
                "Invalid characters in {}: {}",
                context,
                identifier
            ));
        }

        // Check for SQL keywords (basic list)
        // Only reject if the identifier is exactly the keyword or contains it as a whole word
        let sql_keywords = [
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "DROP",
            "CREATE",
            "ALTER",
            "TRUNCATE",
            "EXEC",
            "EXECUTE",
            "UNION",
            "GRANT",
            "REVOKE",
            "COMMIT",
            "ROLLBACK",
            "SAVEPOINT",
        ];

        let upper = identifier.to_uppercase();

        // Check if identifier contains a keyword as a whole word (delimited by underscore, dot, or start/end)
        // This prevents "updated_at" from being rejected while still catching "UPDATE" or "table_UPDATE_column"
        for keyword in &sql_keywords {
            // Check if keyword appears as a whole word:
            // - Exactly matches the keyword
            // - At start followed by underscore or dot
            // - At end preceded by underscore or dot
            // - Surrounded by underscores or dots
            if upper == *keyword
                || upper.starts_with(&format!("{}_", keyword))
                || upper.starts_with(&format!("{}.", keyword))
                || upper.ends_with(&format!("_{}", keyword))
                || upper.ends_with(&format!(".{}", keyword))
                || upper.contains(&format!("_{}_", keyword))
                || upper.contains(&format!("_{}.", keyword))
                || upper.contains(&format!(".{}_", keyword))
                || upper.contains(&format!(".{}.", keyword))
            {
                return Err(anyhow::anyhow!(
                    "SQL keyword detected in {}: {}",
                    context,
                    identifier
                ));
            }
        }

        Ok(())
    }

    /// Validate filter value based on operator
    fn validate_filter_value(&self, operator: &str, value: &serde_json::Value) -> Result<()> {
        match operator.to_uppercase().as_str() {
            "IN" | "NOT IN" => {
                if !value.is_array() {
                    return Err(anyhow::anyhow!(
                        "Value for {} operator must be an array",
                        operator
                    ));
                }

                let arr = value.as_array().unwrap();
                if arr.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Array for {} operator cannot be empty",
                        operator
                    ));
                }

                if arr.len() > 1000 {
                    return Err(anyhow::anyhow!(
                        "Array for {} operator too large (max 1000 items)",
                        operator
                    ));
                }
            }
            "IS NULL" | "IS NOT NULL" => {
                if !value.is_null() {
                    return Err(anyhow::anyhow!(
                        "Value for {} operator must be null",
                        operator
                    ));
                }
            }
            "LIKE" | "NOT LIKE" => {
                if !value.is_string() {
                    return Err(anyhow::anyhow!(
                        "Value for {} operator must be a string",
                        operator
                    ));
                }

                let pattern = value.as_str().unwrap();
                // Basic check for excessive wildcards
                let wildcard_count = pattern.chars().filter(|&c| c == '%' || c == '_').count();
                if wildcard_count > 10 {
                    return Err(anyhow::anyhow!("Too many wildcards in LIKE pattern"));
                }
            }
            _ => {
                // For other operators, just ensure it's a valid JSON value
                if value.is_object() {
                    return Err(anyhow::anyhow!(
                        "Complex objects not allowed in filter values"
                    ));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_query() {
        let config = ORMGatewayConfig::default();
        let validator = QueryValidator::new(config);

        let query = ORMQuery {
            operation: "select".to_string(),
            table: "users".to_string(),
            schema: Some("public".to_string()),
            columns: Some(vec!["id".to_string(), "name".to_string()]),
            filters: Some(vec![super::super::QueryFilter {
                column: "id".to_string(),
                operator: "=".to_string(),
                value: json!(1),
            }]),
            order_by: Some(vec![super::super::OrderBy {
                column: "name".to_string(),
                direction: "ASC".to_string(),
            }]),
            limit: Some(100),
            offset: None,
            aggregations: None,
            values: None,
            set_values: None,
        };

        assert!(validator.validate(&query).is_ok());
    }

    #[test]
    fn test_sql_injection_prevention() {
        let config = ORMGatewayConfig::default();
        let validator = QueryValidator::new(config);

        // Test SQL injection in table name
        let query = ORMQuery {
            operation: "select".to_string(),
            table: "users; DROP TABLE users;".to_string(),
            schema: None,
            columns: None,
            filters: None,
            order_by: None,
            limit: None,
            offset: None,
            aggregations: None,
            values: None,
            set_values: None,
        };

        assert!(validator.validate(&query).is_err());
    }
}
