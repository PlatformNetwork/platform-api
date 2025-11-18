//! Node operations

use super::rows::NodeRow;
use super::PostgresStorageBackend;
use anyhow::Result;
use chrono::Utc;
use platform_api_models::*;
use uuid::Uuid;

impl PostgresStorageBackend {
    /// List nodes with pagination
    pub async fn list_nodes_impl(
        &self,
        pool_id: Option<Uuid>,
        page: u32,
        per_page: u32,
    ) -> Result<NodeListResponse> {
        let offset = (page.saturating_sub(1)) as i64 * per_page as i64;
        let limit = per_page as i64;

        // Count total nodes
        let total = if let Some(pool_id) = pool_id {
            sqlx::query_scalar::<_, i64>(r#"SELECT COUNT(*) FROM nodes WHERE pool_id = $1"#)
                .bind(pool_id)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0)
        } else {
            sqlx::query_scalar::<_, i64>(r#"SELECT COUNT(*) FROM nodes"#)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0)
        };

        // Fetch nodes
        let rows = if let Some(pool_id) = pool_id {
            sqlx::query_as::<_, NodeRow>(r#"
                SELECT id, pool_id, name, vmm_url, capacity, health, metadata, created_at, updated_at
                FROM nodes
                WHERE pool_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
            "#)
                .bind(pool_id)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query_as::<_, NodeRow>(r#"
                SELECT id, pool_id, name, vmm_url, capacity, health, metadata, created_at, updated_at
                FROM nodes
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
            "#)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?
        };

        let nodes = rows
            .into_iter()
            .map(|row| Node {
                id: row.id,
                pool_id: row.pool_id,
                name: row.name,
                vmm_url: row.vmm_url,
                capacity: serde_json::from_value(row.capacity)
                    .unwrap_or_else(|_| NodeCapacity::default()),
                health: serde_json::from_value(row.health).unwrap_or_else(|_| NodeHealth {
                    status: HealthStatus::Unknown,
                    last_heartbeat: Utc::now(),
                    vmm_reachable: false,
                    guest_agent_reachable: false,
                    error: Some("Failed to parse health data".to_string()),
                }),
                metadata: row.metadata.unwrap_or_default(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();

        Ok(NodeListResponse {
            nodes,
            total: total as u64,
            page,
            per_page,
        })
    }

    /// Get node by ID
    pub async fn get_node_impl(&self, id: Uuid) -> Result<Node> {
        let row = sqlx::query_as::<_, NodeRow>(
            r#"
            SELECT id, pool_id, name, vmm_url, capacity, health, metadata, created_at, updated_at
            FROM nodes
            WHERE id = $1
        "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Node not found"))?;

        Ok(Node {
            id: row.id,
            pool_id: row.pool_id,
            name: row.name,
            vmm_url: row.vmm_url,
            capacity: serde_json::from_value(row.capacity)
                .unwrap_or_else(|_| NodeCapacity::default()),
            health: serde_json::from_value(row.health).unwrap_or_else(|_| NodeHealth {
                status: HealthStatus::Unknown,
                last_heartbeat: Utc::now(),
                vmm_reachable: false,
                guest_agent_reachable: false,
                error: Some("Failed to parse health data".to_string()),
            }),
            metadata: row.metadata.unwrap_or_default(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// Add new node to pool
    pub async fn add_node_impl(&self, pool_id: Uuid, request: AddNodeRequest) -> Result<Node> {
        // Verify pool exists
        self.get_pool_impl(pool_id).await?;

        let capacity_json = serde_json::to_value(&request.capacity)?;
        let health_json = serde_json::to_value(&NodeHealth {
            status: HealthStatus::Unknown,
            last_heartbeat: Utc::now(),
            vmm_reachable: false,
            guest_agent_reachable: false,
            error: None,
        })?;
        let metadata_json = request.metadata.unwrap_or(serde_json::json!({}));

        let row = sqlx::query_as::<_, NodeRow>(
            r#"
            INSERT INTO nodes (pool_id, name, vmm_url, capacity, health, metadata)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, pool_id, name, vmm_url, capacity, health, metadata, created_at, updated_at
        "#,
        )
        .bind(pool_id)
        .bind(&request.name)
        .bind(&request.vmm_url)
        .bind(&capacity_json)
        .bind(&health_json)
        .bind(&metadata_json)
        .fetch_one(&self.pool)
        .await?;

        Ok(Node {
            id: row.id,
            pool_id: row.pool_id,
            name: row.name,
            vmm_url: row.vmm_url,
            capacity: serde_json::from_value(row.capacity)
                .unwrap_or_else(|_| NodeCapacity::default()),
            health: serde_json::from_value(row.health).unwrap_or_else(|_| NodeHealth {
                status: HealthStatus::Unknown,
                last_heartbeat: Utc::now(),
                vmm_reachable: false,
                guest_agent_reachable: false,
                error: Some("Failed to parse health data".to_string()),
            }),
            metadata: row.metadata.unwrap_or_default(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// Update node
    pub async fn update_node_impl(&self, id: Uuid, request: UpdateNodeRequest) -> Result<Node> {
        // First get the existing node
        let existing = self.get_node_impl(id).await?;

        // Apply updates
        let name = request.name.unwrap_or(existing.name);
        let vmm_url = request.vmm_url.unwrap_or(existing.vmm_url);
        let capacity = request.capacity.unwrap_or(existing.capacity);
        let metadata = request.metadata.unwrap_or(existing.metadata);

        let capacity_json = serde_json::to_value(&capacity)?;
        let metadata_json = serde_json::to_value(&metadata)?;

        let row = sqlx::query_as::<_, NodeRow>(
            r#"
            UPDATE nodes
            SET name = $2, vmm_url = $3, capacity = $4, metadata = $5
            WHERE id = $1
            RETURNING id, pool_id, name, vmm_url, capacity, health, metadata, created_at, updated_at
        "#,
        )
        .bind(id)
        .bind(&name)
        .bind(&vmm_url)
        .bind(&capacity_json)
        .bind(&metadata_json)
        .fetch_one(&self.pool)
        .await?;

        Ok(Node {
            id: row.id,
            pool_id: row.pool_id,
            name: row.name,
            vmm_url: row.vmm_url,
            capacity: serde_json::from_value(row.capacity)
                .unwrap_or_else(|_| NodeCapacity::default()),
            health: serde_json::from_value(row.health).unwrap_or_else(|_| NodeHealth {
                status: HealthStatus::Unknown,
                last_heartbeat: Utc::now(),
                vmm_reachable: false,
                guest_agent_reachable: false,
                error: Some("Failed to parse health data".to_string()),
            }),
            metadata: row.metadata.unwrap_or_default(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// Delete node
    pub async fn delete_node_impl(&self, id: Uuid) -> Result<()> {
        let result = sqlx::query("DELETE FROM nodes WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!("Node not found"));
        }

        Ok(())
    }
}
