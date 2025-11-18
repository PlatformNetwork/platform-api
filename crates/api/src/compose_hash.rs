use anyhow::{Context, Result};
use serde_yaml;
use sha2::{Digest, Sha256};

/// Calculate SHA256 hash from docker-compose file content
///
/// This function calculates the hash using the dstack-specific normalization:
/// - Parses the content as JSON
/// - Sets docker_config to an empty object
/// - Serializes to compact JSON
/// - Calculates SHA256 hash
///
/// This hash is used to verify that validators are running the expected
/// docker-compose configuration stored in vm_compose_configs.
pub fn calculate_compose_hash_from_content(content: &str) -> Result<String> {
    // Normalize the content for consistent hashing
    let normalized = normalize_compose_content(content)?;

    // Calculate SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let hash = hasher.finalize();
    let hash_hex = hex::encode(hash);

    tracing::info!("Calculated compose hash: {}", hash_hex);

    Ok(hash_hex)
}

/// Normalize docker-compose content for consistent hashing.
///
/// This function implements the dstack-specific normalization for compose manifests.
/// It handles both YAML (from vm_compose_configs database) and JSON formats.
/// The content is converted to JSON, then `docker_config` is set to an empty object,
/// and the result is serialized as compact JSON.
fn normalize_compose_content(content: &str) -> Result<String> {
    // Try to parse as JSON first (expected format for dstack compose manifests)
    let mut json_value: serde_json::Value = match serde_json::from_str(content) {
        Ok(value) => value,
        Err(_) => {
            // If JSON parsing fails, try YAML (format stored in vm_compose_configs)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_compose_content_json() {
        let input = r#"
        {
            "version": "3.8",
            "services": {
                "api": {
                    "image": "nginx:latest",
                    "ports": ["3000:3000"]
                }
            },
            "docker_config": {
                "user": "test"
            }
        }
"#;

        let normalized = normalize_compose_content(input).unwrap();
        let expected = r#"{"docker_config":{},"services":{"api":{"image":"nginx:latest","ports":["3000:3000"]}},"version":"3.8"}"#;

        // Note: serde_json does not guarantee key order, so we parse back to Value and compare.
        let normalized_val: serde_json::Value = serde_json::from_str(&normalized).unwrap();
        let expected_val: serde_json::Value = serde_json::from_str(expected).unwrap();

        assert_eq!(normalized_val, expected_val);
    }

    #[test]
    fn test_compose_hash_consistency() {
        // Same content should produce same hash
        let content1 = r#"{"services":{"api":{"image":"nginx"}},"version":"3.8"}"#;
        let content2 = r#"{"version":"3.8","services":{"api":{"image":"nginx"}},"docker_config":{"auths":{}}}"#;

        let hash1 = calculate_compose_hash_from_content(content1).unwrap();
        let hash2 = calculate_compose_hash_from_content(content2).unwrap();

        // Different content but should produce same hash after normalization of docker_config
        let content3 =
            r#"{"version":"3.8","services":{"api":{"image":"nginx"}},"docker_config":{}}"#;
        let hash3 = calculate_compose_hash_from_content(content3).unwrap();

        // hash1 and hash2 should be different because content1 does not have docker_config, so it is added.
        // content2 has docker_config, which is replaced.
        // So we compare hash2 and hash3
        assert_ne!(hash1, hash2);

        let content4 = r#"{"version":"3.8","services":{"api":{"image":"nginx"}},"docker_config":{"some":"value"}}"#;
        let hash4 = calculate_compose_hash_from_content(content4).unwrap();
        assert_eq!(hash2, hash3);
        assert_eq!(hash3, hash4);
    }

    #[test]
    fn test_normalize_compose_content_yaml() {
        // Test YAML format (as stored in vm_compose_configs)
        let yaml_input = r#"
version: '3.8'
services:
  validator:
    image: validator:latest
    container_name: validator_vm
    restart: unless-stopped
    ports:
      - "8080:8080"
    environment:
      - NODE_ENV=production
volumes:
  validator_data:
    driver: local
"#;

        // Should successfully parse YAML and convert to JSON
        let normalized = normalize_compose_content(yaml_input).unwrap();

        // Should be valid JSON
        let json_value: serde_json::Value =
            serde_json::from_str(&normalized).expect("Normalized output should be valid JSON");

        // Should have docker_config set to empty object
        assert!(json_value.get("docker_config").is_some());
        assert_eq!(json_value["docker_config"], serde_json::json!({}));

        // Should have services
        assert!(json_value.get("services").is_some());
    }
}
