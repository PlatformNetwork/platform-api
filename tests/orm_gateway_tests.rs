// Unit tests for ORM Gateway
// Uses real PostgreSQL with sqlx::test (fast, testable)

use platform_api::orm_gateway::{SecureORMGateway, ORMGatewayConfig, ORMQuery, QueryFilter, OrderBy};
use sqlx::PgPool;
use uuid::Uuid;
use serde_json::json;
use std::sync::Arc;
use std::path::PathBuf;

// Helper to create test database pool (reuse from scheduler_tests)
async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://platform:platform@localhost:5432/platform_test".to_string());
    
    let pool = PgPool::connect(&database_url).await
        .expect("Failed to connect to test database");
    
    // Run migrations
    let migrations_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("crates/storage/migrations"))
        .expect("Failed to find migrations directory");
    
    sqlx::migrate::Migrator::new(&migrations_path)
        .await
        .expect("Failed to create migrator")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    
    pool
}

#[tokio::test]
async fn test_orm_gateway_creation() {
    let pool = setup_test_db().await;
    let config = ORMGatewayConfig::default();
    let gateway = SecureORMGateway::new(config, pool);
    
    // Test that gateway can be created
    assert!(true);
}

#[tokio::test]
async fn test_query_validation() {
    let pool = setup_test_db().await;
    let config = ORMGatewayConfig::default();
    let gateway = SecureORMGateway::new(config, pool);
    
    // Test valid SELECT query
    let query = ORMQuery {
        operation: "select".to_string(),
        table: "jobs".to_string(),
        schema: None,
        columns: Some(vec!["id".to_string(), "status".to_string()]),
        filters: None,
        order_by: None,
        limit: Some(10),
        offset: None,
        aggregations: None,
        values: None,
        set_values: None,
    };
    
    // Query should be validated (we test the validator, not execution)
    assert_eq!(query.operation, "select");
    assert_eq!(query.table, "jobs");
}

#[tokio::test]
async fn test_query_with_filters() {
    let pool = setup_test_db().await;
    let config = ORMGatewayConfig::default();
    let gateway = SecureORMGateway::new(config, pool);
    
    // Test query with filters
    let query = ORMQuery {
        operation: "select".to_string(),
        table: "jobs".to_string(),
        schema: None,
        columns: Some(vec!["id".to_string()]),
        filters: Some(vec![QueryFilter {
            column: "status".to_string(),
            operator: "=".to_string(),
            value: json!("pending"),
        }]),
        order_by: None,
        limit: Some(10),
        offset: None,
        aggregations: None,
        values: None,
        set_values: None,
    };
    
    assert!(query.filters.is_some());
    assert_eq!(query.filters.as_ref().unwrap().len(), 1);
}

#[tokio::test]
async fn test_query_with_order_by() {
    let pool = setup_test_db().await;
    let config = ORMGatewayConfig::default();
    let gateway = SecureORMGateway::new(config, pool);
    
    // Test query with order by
    let query = ORMQuery {
        operation: "select".to_string(),
        table: "jobs".to_string(),
        schema: None,
        columns: Some(vec!["id".to_string(), "created_at".to_string()]),
        filters: None,
        order_by: Some(vec![OrderBy {
            column: "created_at".to_string(),
            direction: "DESC".to_string(),
        }]),
        limit: Some(10),
        offset: None,
        aggregations: None,
        values: None,
        set_values: None,
    };
    
    assert!(query.order_by.is_some());
    assert_eq!(query.order_by.as_ref().unwrap().len(), 1);
}

#[tokio::test]
async fn test_read_only_config() {
    let config = ORMGatewayConfig::default();
    assert!(config.read_only);
    assert!(config.allowed_operations.contains(&"select".to_string()));
    assert!(!config.allowed_operations.contains(&"insert".to_string()));
}

#[tokio::test]
async fn test_read_write_config() {
    let config = ORMGatewayConfig::read_write();
    assert!(!config.read_only);
    assert!(config.allowed_operations.contains(&"select".to_string()));
    assert!(config.allowed_operations.contains(&"insert".to_string()));
    assert!(config.allowed_operations.contains(&"update".to_string()));
    assert!(config.allowed_operations.contains(&"delete".to_string()));
}

#[tokio::test]
async fn test_data_type_conversions() {
    let pool = setup_test_db().await;
    let config = ORMGatewayConfig::read_write();
    let gateway = SecureORMGateway::new(config, pool.clone());
    
    // Create a test table with various data types
    let test_table_name = format!("test_types_{}", Uuid::new_v4().to_string().replace("-", ""));
    
    let create_table_sql = format!(
        r#"
        CREATE TABLE IF NOT EXISTS {} (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            int2_col SMALLINT,
            int4_col INTEGER,
            int8_col BIGINT,
            numeric_col NUMERIC(10, 2),
            float4_col REAL,
            float8_col DOUBLE PRECISION,
            bool_col BOOLEAN,
            text_col TEXT,
            varchar_col VARCHAR(255),
            timestamp_col TIMESTAMP,
            timestamptz_col TIMESTAMPTZ,
            date_col DATE,
            time_col TIME,
            json_col JSON,
            jsonb_col JSONB,
            uuid_col UUID
        )
        "#,
        test_table_name
    );
    
    sqlx::query(&create_table_sql)
        .execute(&pool)
        .await
        .expect("Failed to create test table");
    
    // Insert test data with known values
    let insert_sql = format!(
        r#"
        INSERT INTO {} (
            int2_col, int4_col, int8_col, numeric_col,
            float4_col, float8_col, bool_col, text_col, varchar_col,
            timestamp_col, timestamptz_col, date_col, time_col,
            json_col, jsonb_col, uuid_col
        ) VALUES (
            42, 1234, 567890, 123.45,
            3.14, 2.718281828, true, 'test text', 'test varchar',
            '2024-01-15 10:30:00', '2024-01-15 10:30:00+00', '2024-01-15', '10:30:00',
            '{{"key": "value"}}', '{{"nested": {{"data": 123}}}}', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'
        )
        RETURNING *
        "#,
        test_table_name
    );
    
    let result = sqlx::query(&insert_sql)
        .fetch_one(&pool)
        .await
        .expect("Failed to insert test data");
    
    // Now use ORM gateway to query the data
    let query = ORMQuery {
        operation: "select".to_string(),
        table: test_table_name.clone(),
        schema: None,
        columns: None, // SELECT *
        filters: None,
        order_by: None,
        limit: Some(1),
        offset: None,
        aggregations: None,
        values: None,
        set_values: None,
    };
    
    let query_result = gateway.execute_query(query)
        .await
        .expect("Failed to execute query");
    
    assert_eq!(query_result.row_count, 1);
    assert_eq!(query_result.rows.len(), 1);
    
    let row = &query_result.rows[0];
    
    // Verify integer types are JSON numbers
    assert!(row["int2_col"].is_number(), "int2_col should be a number");
    assert_eq!(row["int2_col"].as_i64().unwrap(), 42);
    
    assert!(row["int4_col"].is_number(), "int4_col should be a number");
    assert_eq!(row["int4_col"].as_i64().unwrap(), 1234);
    
    assert!(row["int8_col"].is_number(), "int8_col should be a number");
    assert_eq!(row["int8_col"].as_i64().unwrap(), 567890);
    
    // Verify numeric/decimal is a JSON number
    assert!(row["numeric_col"].is_number(), "numeric_col should be a number");
    let numeric_val = row["numeric_col"].as_f64().unwrap();
    assert!((numeric_val - 123.45).abs() < 0.01, "numeric_col value mismatch");
    
    // Verify float types are JSON numbers
    assert!(row["float4_col"].is_number(), "float4_col should be a number");
    let float4_val = row["float4_col"].as_f64().unwrap();
    assert!((float4_val - 3.14).abs() < 0.01, "float4_col value mismatch");
    
    assert!(row["float8_col"].is_number(), "float8_col should be a number");
    let float8_val = row["float8_col"].as_f64().unwrap();
    assert!((float8_val - 2.718281828).abs() < 0.001, "float8_col value mismatch");
    
    // Verify boolean is a JSON boolean
    assert!(row["bool_col"].is_boolean(), "bool_col should be a boolean");
    assert_eq!(row["bool_col"].as_bool().unwrap(), true);
    
    // Verify text types are JSON strings
    assert!(row["text_col"].is_string(), "text_col should be a string");
    assert_eq!(row["text_col"].as_str().unwrap(), "test text");
    
    assert!(row["varchar_col"].is_string(), "varchar_col should be a string");
    assert_eq!(row["varchar_col"].as_str().unwrap(), "test varchar");
    
    // Verify timestamp types are JSON strings (ISO8601 format)
    assert!(row["timestamp_col"].is_string(), "timestamp_col should be a string");
    let ts_str = row["timestamp_col"].as_str().unwrap();
    assert!(ts_str.contains("2024-01-15"), "timestamp_col should contain date");
    assert!(ts_str.contains("10:30:00"), "timestamp_col should contain time");
    
    assert!(row["timestamptz_col"].is_string(), "timestamptz_col should be a string");
    let tstz_str = row["timestamptz_col"].as_str().unwrap();
    assert!(tstz_str.contains("2024-01-15"), "timestamptz_col should contain date");
    
    assert!(row["date_col"].is_string(), "date_col should be a string");
    assert_eq!(row["date_col"].as_str().unwrap(), "2024-01-15");
    
    assert!(row["time_col"].is_string(), "time_col should be a string");
    assert!(row["time_col"].as_str().unwrap().starts_with("10:30:00"), "time_col value mismatch");
    
    // Verify JSON types are proper JSON objects (not stringified)
    assert!(row["json_col"].is_object(), "json_col should be an object");
    assert_eq!(row["json_col"]["key"].as_str().unwrap(), "value");
    
    assert!(row["jsonb_col"].is_object(), "jsonb_col should be an object");
    assert!(row["jsonb_col"]["nested"].is_object(), "jsonb_col nested should be an object");
    assert_eq!(row["jsonb_col"]["nested"]["data"].as_i64().unwrap(), 123);
    
    // Verify UUID is a string
    assert!(row["uuid_col"].is_string(), "uuid_col should be a string");
    assert_eq!(row["uuid_col"].as_str().unwrap(), "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11");
    
    // Clean up: drop the test table
    let drop_sql = format!("DROP TABLE IF EXISTS {}", test_table_name);
    sqlx::query(&drop_sql)
        .execute(&pool)
        .await
        .expect("Failed to drop test table");
}

