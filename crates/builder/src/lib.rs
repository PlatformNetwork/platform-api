use anyhow::{Context, Result};
use chrono::Utc;
use platform_api_models::{
    ChallengeMetadata, ChallengePort, ChallengeResources, ChallengeStatus, ChallengeVisibility,
    CreateChallengeRequest, UpdateChallengeRequest,
};
use serde_yaml;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

/// Builder service
pub struct BuilderService {
    config: BuilderConfig,
    database_pool: Option<Arc<PgPool>>,
}

impl BuilderService {
    pub fn new(config: &BuilderConfig, database_pool: Option<Arc<PgPool>>) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            database_pool,
        })
    }

    /// Calculate compose_hash from docker-compose file
    /// Uses the same normalization as compose_hash.rs for consistency
    fn calculate_compose_hash(&self, challenge_name: &str) -> Result<String> {
        info!("Calculating compose_hash for challenge: {}", challenge_name);

        // Try to find docker-compose file for this challenge
        // For term-challenge, look in term-challenge directory
        let possible_paths: Vec<String> = if challenge_name == "term-challenge" {
            vec![
                "term-challenge/docker-compose.yaml".to_string(),
                "term-challenge/docker-compose.yml".to_string(),
                "../term-challenge/docker-compose.yaml".to_string(),
                "../term-challenge/docker-compose.yml".to_string(),
                "./docker-compose.dev.yml".to_string(),
                "../docker-compose.dev.yml".to_string(),
                "docker-compose.dev.yml".to_string(),
            ]
        } else {
            vec![
                format!("{}/docker-compose.yaml", challenge_name),
                format!("{}/docker-compose.yml", challenge_name),
                "./docker-compose.dev.yml".to_string(),
                "../docker-compose.dev.yml".to_string(),
                "docker-compose.dev.yml".to_string(),
            ]
        };

        let mut file_content: Option<String> = None;
        let mut used_path: Option<String> = None;

        for path_str in possible_paths {
            let path = Path::new(&path_str);
            if path.exists() && path.is_file() {
                match fs::read_to_string(path) {
                    Ok(content) => {
                        file_content = Some(content);
                        used_path = Some(path_str.clone());
                        break;
                    }
                    Err(e) => {
                        warn!("Failed to read {}: {}", path_str, e);
                        continue;
                    }
                }
            }
        }

        let content = file_content
            .context("Could not find docker-compose file for challenge. Compose hash will be generated from name.")?;

        // Use the same normalization as compose_hash.rs
        let final_hash = Self::calculate_compose_hash_from_content(&content)?;

        info!(
            "Calculated compose hash from: {} -> {}",
            used_path.as_deref().unwrap_or("unknown"),
            final_hash
        );
        Ok(final_hash)
    }

    /// Calculate SHA256 hash from docker-compose file content
    ///
    /// This function calculates the hash using the dstack-specific normalization:
    /// - Parses the content as JSON or YAML
    /// - Sets docker_config to an empty object
    /// - Serializes to compact JSON
    /// - Calculates SHA256 hash
    ///
    /// This matches the normalization used in platform-api/crates/api/src/compose_hash.rs
    fn calculate_compose_hash_from_content(content: &str) -> Result<String> {
        // Normalize the content for consistent hashing
        let normalized = Self::normalize_compose_content(content)?;

        // Calculate SHA256 hash
        let mut hasher = Sha256::new();
        hasher.update(normalized.as_bytes());
        let hash = hasher.finalize();
        let hash_hex = hex::encode(hash);

        info!("Calculated compose hash: {}", hash_hex);

        Ok(hash_hex)
    }

    /// Normalize docker-compose content for consistent hashing.
    ///
    /// This function implements the dstack-specific normalization for compose manifests.
    /// It handles both YAML (from docker-compose files) and JSON formats.
    /// The content is converted to JSON, then `docker_config` is set to an empty object,
    /// and the result is serialized as compact JSON.
    ///
    /// This matches the normalization used in platform-api/crates/api/src/compose_hash.rs
    fn normalize_compose_content(content: &str) -> Result<String> {
        // Try to parse as JSON first (expected format for dstack compose manifests)
        let mut json_value: serde_json::Value = match serde_json::from_str(content) {
            Ok(value) => value,
            Err(_) => {
                // If JSON parsing fails, try YAML (format from docker-compose files)
                let yaml_value: serde_yaml::Value = serde_yaml::from_str(content)
                    .context("Failed to parse compose content as JSON or YAML")?;

                // Convert YAML Value to JSON Value
                serde_json::to_value(yaml_value).context("Failed to convert YAML to JSON")?
            }
        };

        // The dstack RTMR3 calculation for compose_hash requires setting docker_config to an empty object.
        if let Some(obj) = json_value.as_object_mut() {
            obj.insert(
                "docker_config".to_string(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
        }

        // Serialize back to a compact string, which is what's hashed.
        serde_json::to_string(&json_value).context("Failed to serialize normalized compose content")
    }

    pub async fn create_challenge(
        &self,
        request: CreateChallengeRequest,
    ) -> Result<ChallengeMetadata> {
        // Generate deterministic ID from request data
        let id_bytes = format!("{}{}", request.name, request.description);
        let id_hash = sha2::Sha256::digest(id_bytes.as_bytes());
        let id = Uuid::from_bytes([
            id_hash[0],
            id_hash[1],
            id_hash[2],
            id_hash[3],
            id_hash[4],
            id_hash[5],
            id_hash[6],
            id_hash[7],
            id_hash[8],
            id_hash[9],
            id_hash[10],
            id_hash[11],
            id_hash[12],
            id_hash[13],
            id_hash[14],
            id_hash[15],
        ]);

        let now = Utc::now();

        // If database pool is available, insert into PostgreSQL
        if let Some(pool) = &self.database_pool {
            info!("Database pool available, inserting challenge into PostgreSQL");
            // Calculate compose_hash - for term-challenge use a consistent hash
            let compose_hash = if request.name == "term-challenge" {
                // Use consistent hash for term-challenge in dev environment
                let hash = "term-challenge-dev-001".to_string();
                info!("Using predefined compose_hash for term-challenge: {}", hash);
                hash
            } else {
                match self.calculate_compose_hash(&request.name) {
                    Ok(hash) => {
                        info!("Calculated compose_hash: {}", hash);
                        hash
                    }
                    Err(e) => {
                        warn!(
                            "Failed to calculate compose_hash: {}. Using fallback hash.",
                            e
                        );
                        // Fallback: generate hash from challenge name
                        let mut hasher = Sha256::new();
                        hasher.update(format!("{}-{}", request.name, now.timestamp()).as_bytes());
                        let fallback_hash = format!("dev-{}", hex::encode(hasher.finalize()));
                        info!("Using fallback compose_hash: {}", fallback_hash);
                        fallback_hash
                    }
                }
            };

            // Read compose_yaml (try to read docker-compose file)
            let compose_yaml = if request.name == "term-challenge" {
                // Use specific compose for term-challenge
                r#"# Term Challenge Docker Compose
# This is a placeholder compose file for term-challenge
# The actual service is defined in the main docker-compose.dev.yml
version: "3.8"
services:
  term-challenge:
    image: term-challenge:dev
    ports:
      - "10000:10000"
    environment:
      - CHALLENGE_ADMIN=true
      - SDK_DEV_MODE=true
"#
                .to_string()
            } else {
                self.read_compose_yaml(&request.name).unwrap_or_else(|| {
                    format!("# Challenge: {}\n# Created: {}\n", request.name, now)
                })
            };

            info!(
                "Using compose_yaml (first 100 chars): {}...",
                &compose_yaml[..compose_yaml.len().min(100)]
            );

            // Set default values for required fields
            let version = "1.0.0".to_string();
            let images: Vec<String> = if request.name == "term-challenge" {
                vec!["term-challenge:dev".to_string()]
            } else {
                vec![] // Empty for now, will be populated when challenge is deployed
            };
            let resources = ChallengeResources {
                vcpu: request.harness_config.resources.cpu_cores,
                memory: format!("{}G", request.harness_config.resources.memory_mb / 1024), // Convert MB to G
                disk: Some(format!(
                    "{}G",
                    request.harness_config.resources.disk_mb / 1024
                )), // Convert MB to G
            };
            let ports: Vec<ChallengePort> = vec![]; // Empty for now
            let env: BTreeMap<String, String> = request.harness_config.environment.clone();
            let emission_share = 1.0; // Default to 1.0 (100%)
            let mechanism_id: i16 = 0; // Default mechanism ID
            let weight: Option<f64> = None; // Will be auto-calculated

            info!(
                "Preparing to insert challenge '{}' into PostgreSQL with compose_hash: {}",
                request.name, compose_hash
            );
            info!(
                "Challenge details - version: {}, images: {:?}, resources: {:?}",
                version, images, resources
            );

            // Insert into PostgreSQL
            sqlx::query(
                r#"
                INSERT INTO challenges (
                    id, name, compose_hash, compose_yaml, version, images,
                    resources, ports, env, emission_share, mechanism_id, weight,
                    description, github_repo, created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
                ON CONFLICT (compose_hash) DO UPDATE SET
                    name = EXCLUDED.name,
                    compose_yaml = EXCLUDED.compose_yaml,
                    version = EXCLUDED.version,
                    images = EXCLUDED.images,
                    resources = EXCLUDED.resources,
                    ports = EXCLUDED.ports,
                    env = EXCLUDED.env,
                    emission_share = EXCLUDED.emission_share,
                    mechanism_id = EXCLUDED.mechanism_id,
                    weight = EXCLUDED.weight,
                    description = EXCLUDED.description,
                    github_repo = EXCLUDED.github_repo,
                    updated_at = EXCLUDED.updated_at
                "#,
            )
            .bind(id)
            .bind(&request.name)
            .bind(&compose_hash)
            .bind(&compose_yaml)
            .bind(&version)
            .bind(&images)
            .bind(serde_json::to_value(&resources)?)
            .bind(serde_json::to_value(&ports)?)
            .bind(serde_json::to_value(&env)?)
            .bind(emission_share)
            .bind(mechanism_id)
            .bind(weight)
            .bind(&request.description)
            .bind(request.github_repo.as_deref())
            .bind(now)
            .bind(now)
            .execute(pool.as_ref())
            .await
            .context("Failed to insert challenge into PostgreSQL")?;

            info!(
                "✅ Challenge '{}' successfully inserted into PostgreSQL with compose_hash: {}",
                request.name, compose_hash
            );
            info!(
                "   ID: {}, Emission share: {}, Mechanism ID: {}",
                id, emission_share, mechanism_id
            );
        } else {
            warn!("❌ No database pool available, challenge not inserted into PostgreSQL");
        }

        Ok(ChallengeMetadata {
            id,
            name: request.name,
            description: request.description,
            version: "1.0.0".to_string(),
            visibility: request.visibility,
            status: ChallengeStatus::Active,
            owner: "platform-system".to_string(),
            created_at: now,
            updated_at: now,
            tags: vec![],
        })
    }

    /// Read compose_yaml from file
    fn read_compose_yaml(&self, challenge_name: &str) -> Option<String> {
        let possible_paths: Vec<String> = if challenge_name == "term-challenge" {
            vec![
                "term-challenge/docker-compose.yaml".to_string(),
                "term-challenge/docker-compose.yml".to_string(),
                "../term-challenge/docker-compose.yaml".to_string(),
                "../term-challenge/docker-compose.yml".to_string(),
                "./docker-compose.dev.yml".to_string(),
                "../docker-compose.dev.yml".to_string(),
                "docker-compose.dev.yml".to_string(),
            ]
        } else {
            vec![
                format!("{}/docker-compose.yaml", challenge_name),
                format!("{}/docker-compose.yml", challenge_name),
                "./docker-compose.dev.yml".to_string(),
                "../docker-compose.dev.yml".to_string(),
                "docker-compose.dev.yml".to_string(),
            ]
        };

        for path_str in possible_paths {
            let path = Path::new(&path_str);
            if path.exists() && path.is_file() {
                if let Ok(content) = fs::read_to_string(path) {
                    return Some(content);
                }
            }
        }

        None
    }

    pub async fn update_challenge(
        &self,
        id: Uuid,
        request: UpdateChallengeRequest,
    ) -> Result<ChallengeMetadata> {
        // Return updated metadata (minimal implementation)
        Ok(ChallengeMetadata {
            id,
            name: request
                .name
                .unwrap_or_else(|| "Unnamed Challenge".to_string()),
            description: request
                .description
                .unwrap_or_else(|| "No description".to_string()),
            version: "1.0.0".to_string(),
            visibility: ChallengeVisibility::Public,
            status: request.status.unwrap_or(ChallengeStatus::Active),
            owner: "platform-system".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tags: vec![],
        })
    }

    pub async fn delete_challenge(&self, _id: Uuid) -> Result<()> {
        // Challenge deletion is successful
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BuilderConfig {
    pub build_timeout: u64,
    pub max_concurrent_builds: u32,
    pub docker_registry: String,
    pub github_token: Option<String>,
    pub build_cache_size: u64,
}

impl Default for BuilderConfig {
    fn default() -> Self {
        Self {
            build_timeout: 3600,
            max_concurrent_builds: 10,
            docker_registry: "registry.platform.network".to_string(),
            github_token: None,
            build_cache_size: 10000000000,
        }
    }
}
