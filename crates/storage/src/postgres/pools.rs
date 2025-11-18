//! Pool operations

use super::rows::{NodeCapacityRow, PoolRow};
use super::PostgresStorageBackend;
use anyhow::Result;
use chrono::Utc;
use platform_api_models::*;
use uuid::Uuid;

impl PostgresStorageBackend {
    /// List pools with pagination
    pub async fn list_pools_impl(
        &self,
        validator_hotkey: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Result<PoolListResponse> {
        let offset = (page.saturating_sub(1)) as i64 * per_page as i64;
        let limit = per_page as i64;

        // Count total pools
        let total = if let Some(hotkey) = validator_hotkey {
            sqlx::query_scalar::<_, i64>(
                r#"SELECT COUNT(*) FROM pools WHERE validator_hotkey = $1"#,
            )
            .bind(hotkey)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0)
        } else {
            sqlx::query_scalar::<_, i64>(r#"SELECT COUNT(*) FROM pools"#)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0)
        };

        // Fetch pools
        let rows = if let Some(hotkey) = validator_hotkey {
            sqlx::query_as::<_, PoolRow>(r#"
                SELECT id, validator_hotkey, name, description, autoscale_policy, region, created_at, updated_at
                FROM pools
                WHERE validator_hotkey = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
            "#)
                .bind(hotkey)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query_as::<_, PoolRow>(r#"
                SELECT id, validator_hotkey, name, description, autoscale_policy, region, created_at, updated_at
                FROM pools
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
            "#)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?
        };

        let pools = rows
            .into_iter()
            .map(|row| Pool {
                id: row.id,
                validator_hotkey: row.validator_hotkey,
                name: row.name,
                description: row.description,
                autoscale_policy: serde_json::from_value(row.autoscale_policy)
                    .unwrap_or_else(|_| AutoscalePolicy::default()),
                region: row.region,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();

        Ok(PoolListResponse {
            pools,
            total: total as u64,
            page,
            per_page,
        })
    }

    /// Get pool by ID
    pub async fn get_pool_impl(&self, id: Uuid) -> Result<Pool> {
        let row = sqlx::query_as::<_, PoolRow>(r#"
            SELECT id, validator_hotkey, name, description, autoscale_policy, region, created_at, updated_at
            FROM pools
            WHERE id = $1
        "#)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Pool not found"))?;

        Ok(Pool {
            id: row.id,
            validator_hotkey: row.validator_hotkey,
            name: row.name,
            description: row.description,
            autoscale_policy: serde_json::from_value(row.autoscale_policy)
                .unwrap_or_else(|_| AutoscalePolicy::default()),
            region: row.region,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// Create new pool
    pub async fn create_pool_impl(
        &self,
        validator_hotkey: &str,
        request: CreatePoolRequest,
    ) -> Result<Pool> {
        let autoscale_policy_json =
            serde_json::to_value(request.autoscale_policy.unwrap_or_default())?;

        let row = sqlx::query_as::<_, PoolRow>(r#"
            INSERT INTO pools (validator_hotkey, name, description, autoscale_policy, region)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, validator_hotkey, name, description, autoscale_policy, region, created_at, updated_at
        "#)
            .bind(validator_hotkey)
            .bind(&request.name)
            .bind(request.description.as_deref())
            .bind(&autoscale_policy_json)
            .bind(request.region.as_deref())
            .fetch_one(&self.pool)
            .await?;

        Ok(Pool {
            id: row.id,
            validator_hotkey: row.validator_hotkey,
            name: row.name,
            description: row.description,
            autoscale_policy: serde_json::from_value(row.autoscale_policy)
                .unwrap_or_else(|_| AutoscalePolicy::default()),
            region: row.region,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// Update pool
    pub async fn update_pool_impl(&self, id: Uuid, request: UpdatePoolRequest) -> Result<Pool> {
        // First get the existing pool
        let existing = self.get_pool_impl(id).await?;

        // Apply updates
        let name = request.name.unwrap_or(existing.name);
        let description = request.description.or(existing.description);
        let autoscale_policy = request
            .autoscale_policy
            .unwrap_or(existing.autoscale_policy);
        let region = request.region.or(existing.region);

        let autoscale_policy_json = serde_json::to_value(&autoscale_policy)?;

        let row = sqlx::query_as::<_, PoolRow>(r#"
            UPDATE pools
            SET name = $2, description = $3, autoscale_policy = $4, region = $5
            WHERE id = $1
            RETURNING id, validator_hotkey, name, description, autoscale_policy, region, created_at, updated_at
        "#)
            .bind(id)
            .bind(&name)
            .bind(description.as_deref())
            .bind(&autoscale_policy_json)
            .bind(region.as_deref())
            .fetch_one(&self.pool)
            .await?;

        Ok(Pool {
            id: row.id,
            validator_hotkey: row.validator_hotkey,
            name: row.name,
            description: row.description,
            autoscale_policy: serde_json::from_value(row.autoscale_policy)
                .unwrap_or_else(|_| AutoscalePolicy::default()),
            region: row.region,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// Delete pool
    pub async fn delete_pool_impl(&self, id: Uuid) -> Result<()> {
        let result = sqlx::query("DELETE FROM pools WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!("Pool not found"));
        }

        Ok(())
    }

    /// Get pool capacity summary
    pub async fn get_pool_capacity_impl(&self, pool_id: Uuid) -> Result<PoolCapacitySummary> {
        // Verify pool exists
        self.get_pool_impl(pool_id).await?;

        // Get all nodes for the pool
        let nodes = sqlx::query_as::<_, NodeCapacityRow>(
            r#"
            SELECT capacity, health
            FROM nodes
            WHERE pool_id = $1
        "#,
        )
        .bind(pool_id)
        .fetch_all(&self.pool)
        .await?;

        let mut total_cpu = 0u32;
        let mut available_cpu = 0u32;
        let mut total_memory_gb = 0u32;
        let mut available_memory_gb = 0u32;
        let mut healthy_nodes = 0u32;
        let mut has_tdx = false;
        let mut gpu_count = 0u32;

        for node in &nodes {
            let capacity: NodeCapacity = serde_json::from_value(node.capacity.clone())
                .unwrap_or_else(|_| NodeCapacity::default());
            let health: NodeHealth =
                serde_json::from_value(node.health.clone()).unwrap_or_else(|_| NodeHealth {
                    status: HealthStatus::Unknown,
                    last_heartbeat: Utc::now(),
                    vmm_reachable: false,
                    guest_agent_reachable: false,
                    error: Some("Failed to parse health data".to_string()),
                });

            total_cpu += capacity.total_cpu;
            total_memory_gb += capacity.total_memory_gb;
            available_cpu += capacity.available_cpu;
            available_memory_gb += capacity.available_memory_gb;

            if capacity.has_tdx {
                has_tdx = true;
            }

            gpu_count += capacity.gpu_count;

            if matches!(health.status, HealthStatus::Healthy) {
                healthy_nodes += 1;
            }
        }

        Ok(PoolCapacitySummary {
            pool_id,
            total_nodes: nodes.len() as u32,
            healthy_nodes,
            total_cpu,
            available_cpu,
            total_memory_gb,
            available_memory_gb,
            has_tdx,
            gpu_count,
        })
    }
}
