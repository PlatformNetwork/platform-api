//! Challenge and configuration operations

use super::PostgresStorageBackend;
use crate::CreateBackupRequest;
use anyhow::Result;
use platform_api_models::*;
use uuid::Uuid;

impl PostgresStorageBackend {
    /// List challenges (currently returns empty as challenges are managed in memory)
    pub async fn list_challenges_impl(
        &self,
        page: u32,
        per_page: u32,
        _status: Option<String>,
        _visibility: Option<String>,
    ) -> Result<ChallengeListResponse> {
        // For now, return empty list - challenges are managed in memory via challenge_registry
        Ok(ChallengeListResponse {
            challenges: vec![],
            total: 0,
            page,
            per_page,
        })
    }

    /// Get challenge details
    pub async fn get_challenge_impl(&self, _id: Uuid) -> Result<ChallengeDetailResponse> {
        Err(anyhow::anyhow!("Challenge not found"))
    }

    /// Get subnet configuration
    pub async fn get_subnet_config_impl(&self) -> Result<SubnetConfig> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    /// Update subnet configuration
    pub async fn update_subnet_config_impl(&self, _config: SubnetConfig) -> Result<SubnetConfig> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    /// Validate configuration
    pub async fn validate_config_impl(
        &self,
        _config: &UpdateConfigRequest,
    ) -> Result<ConfigValidationResult> {
        Ok(ConfigValidationResult {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
            suggestions: vec![],
        })
    }

    /// Create configuration backup
    pub async fn create_config_backup_impl(
        &self,
        _request: CreateBackupRequest,
    ) -> Result<ConfigBackup> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    /// Restore configuration from backup
    pub async fn restore_config_impl(&self, _request: RestoreConfigRequest) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    /// List configuration backups
    pub async fn list_config_backups_impl(&self) -> Result<Vec<ConfigBackup>> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    /// Get configuration backup by ID
    pub async fn get_config_backup_impl(&self, _id: Uuid) -> Result<ConfigBackup> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    /// Get configuration change history
    pub async fn get_config_history_impl(&self) -> Result<Vec<ConfigChangeLog>> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    /// Get VM compose configuration
    pub async fn get_vm_compose_config_impl(&self, vm_type: &str) -> Result<VmComposeConfig> {
        use super::rows::VmComposeRow;

        let row = sqlx::query_as::<_, VmComposeRow>(
            r#"
            SELECT id, vm_type, compose_content, description, required_env, created_at, updated_at
            FROM vm_compose_configs
            WHERE vm_type = $1
        "#,
        )
        .bind(vm_type)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("VM compose config not found for type: {}", vm_type))?;

        // Parse required_env from JSONB to Vec<String>
        let required_env: Vec<String> =
            serde_json::from_value(row.required_env).unwrap_or_else(|_| vec![]);

        Ok(VmComposeConfig {
            id: row.id,
            vm_type: row.vm_type,
            compose_content: row.compose_content,
            description: row.description,
            required_env,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}
