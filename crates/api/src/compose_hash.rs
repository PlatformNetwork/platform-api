use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// Calculate SHA256 hash of docker-compose file for security attestation
///
/// This hash is used to verify that all services are running the same
/// docker-compose configuration, ensuring network isolation and security.
pub fn calculate_compose_hash(compose_file: &str) -> Result<String> {
    // Try multiple possible locations for docker-compose file
    let possible_paths = vec![
        compose_file.to_string(),
        format!("/app/{}", compose_file),
        format!("./{}", compose_file),
        format!("../{}", compose_file),
        "/app/docker-compose.yml".to_string(),
        "./docker-compose.yml".to_string(),
        "../docker-compose.yml".to_string(),
        "docker-compose.yml".to_string(),
    ];

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
                    tracing::warn!("Failed to read {}: {}", path_str, e);
                    continue;
                }
            }
        }
    }

    let content = file_content
        .context("Could not find or read docker-compose.yml file. Please ensure COMPOSE_FILE environment variable is set correctly.")?;

    // Normalize the content (remove comments, normalize whitespace)
    // This ensures consistent hashing even if comments change
    let normalized = normalize_compose_content(&content);

    // Calculate SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let hash = hasher.finalize();
    let hash_hex = hex::encode(hash);

    tracing::info!(
        "Calculated compose hash from: {}",
        used_path.as_deref().unwrap_or("unknown")
    );
    tracing::info!("   Compose hash: {}", hash_hex);

    Ok(hash_hex)
}

/// Get compose hash with environment mode prefix
///
/// In dev mode, prefixes hash with "dev-" to ensure isolation from production
pub fn get_compose_hash_with_env_mode() -> Result<String> {
    // Get environment mode (dev or prod)
    let env_mode = std::env::var("ENVIRONMENT_MODE").unwrap_or_else(|_| {
        // Auto-detect: if DEV_MODE is set, use dev
        if std::env::var("DEV_MODE").unwrap_or_else(|_| "false".to_string()) == "true" {
            "dev".to_string()
        } else {
            "prod".to_string()
        }
    });

    // Get compose file path from environment or use default
    let compose_file =
        std::env::var("COMPOSE_FILE").unwrap_or_else(|_| "docker-compose.yml".to_string());

    let hash = calculate_compose_hash(&compose_file)?;

    // Prefix with environment mode to ensure dev/prod isolation
    if env_mode == "dev" {
        Ok(format!("dev-{}", hash))
    } else {
        Ok(hash)
    }
}

/// Normalize docker-compose content for consistent hashing
///
/// Removes comments and normalizes whitespace to ensure same hash
/// for functionally identical configurations
fn normalize_compose_content(content: &str) -> String {
    let mut normalized = String::new();

    for line in content.lines() {
        // Remove comments (lines starting with # or after #)
        let line_trimmed = line.trim();

        // Skip empty lines and pure comment lines
        if line_trimmed.is_empty() || line_trimmed.starts_with('#') {
            continue;
        }

        // Remove inline comments
        let line_without_comment = if let Some(comment_pos) = line_trimmed.find('#') {
            // Check if # is inside a quoted string
            let before_comment = &line_trimmed[..comment_pos];
            let quote_count = before_comment.matches('"').count() % 2;
            if quote_count == 0 {
                // Not in a string, remove comment
                before_comment.trim_end()
            } else {
                // In a string, keep as is
                line_trimmed
            }
        } else {
            line_trimmed
        };

        if !line_without_comment.is_empty() {
            normalized.push_str(line_without_comment);
            normalized.push('\n');
        }
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_compose_content() {
        let input = r#"
version: '3.8'

services:
  # This is a comment
  api:
    image: nginx:latest
    ports:
      - "3000:3000"  # Port mapping
"#;

        let normalized = normalize_compose_content(input);

        // Should not contain comments
        assert!(!normalized.contains('#'));
        // Should contain key content
        assert!(normalized.contains("api:"));
        assert!(normalized.contains("nginx:latest"));
    }

    #[test]
    fn test_compose_hash_consistency() {
        // Same content should produce same hash
        let content1 = "version: '3.8'\nservices:\n  api:\n    image: nginx\n";
        let content2 = "version: '3.8'\nservices:\n  api:\n    image: nginx\n";

        let normalized1 = normalize_compose_content(content1);
        let normalized2 = normalize_compose_content(content2);

        assert_eq!(normalized1, normalized2);
    }
}
