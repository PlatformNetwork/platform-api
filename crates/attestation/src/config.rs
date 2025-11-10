use serde::{Deserialize, Serialize};

/// TDX Configuration with production/dev mode support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TdxConfig {
    /// Whether TEE attestation is enforced (production)
    pub tee_enforced: bool,
    /// Whether dev mode is enabled (mock attestation)
    pub dev_mode: bool,
    /// JWT secret for token signing
    pub jwt_secret: String,
    /// Session timeout in seconds
    pub session_timeout: u64,
    /// Storage encryption key
    pub storage_encryption_key: Option<String>,
    /// KBS encryption key
    pub kbs_encryption_key: Option<String>,
}

impl TdxConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let tee_enforced = std::env::var("TEE_ENFORCED")
            .unwrap_or_else(|_| "true".to_string())
            .to_lowercase()
            == "true";

        let dev_mode = std::env::var("DEV_MODE")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true";

        let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
            if dev_mode {
                "dev-secret-not-for-production".to_string()
            } else {
                panic!("JWT_SECRET must be set in production mode")
            }
        });

        let session_timeout = std::env::var("SESSION_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(86400); // 24 hours default

        let storage_encryption_key = std::env::var("STORAGE_ENCRYPTION_KEY").ok();
        let kbs_encryption_key = std::env::var("KBS_ENCRYPTION_KEY").ok();

        // Validate configuration
        if tee_enforced && !dev_mode {
            // Production mode checks
            if jwt_secret == "disabled-no-jwt" || jwt_secret == "dev-secret-not-for-production" {
                panic!("Invalid JWT_SECRET for production mode");
            }
            if storage_encryption_key.is_none() {
                tracing::warn!("STORAGE_ENCRYPTION_KEY not set in production mode");
            }
            if kbs_encryption_key.is_none() {
                tracing::warn!("KBS_ENCRYPTION_KEY not set in production mode");
            }
        }

        Self {
            tee_enforced,
            dev_mode,
            jwt_secret,
            session_timeout,
            storage_encryption_key,
            kbs_encryption_key,
        }
    }

    /// Check if running in production mode
    pub fn is_production(&self) -> bool {
        self.tee_enforced && !self.dev_mode
    }

    /// Check if running in dev mode
    pub fn is_dev_mode(&self) -> bool {
        self.dev_mode || !self.tee_enforced
    }

    /// Get mode description for logging
    pub fn mode_description(&self) -> &'static str {
        match (self.tee_enforced, self.dev_mode) {
            (true, false) => "Production (TEE enforced)",
            (false, true) => "Development (Mock attestation)",
            (true, true) => "Mixed (TEE with dev fallback)",
            (false, false) => "Development (TEE optional)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_production_mode() {
        std::env::set_var("TEE_ENFORCED", "true");
        std::env::set_var("DEV_MODE", "false");
        std::env::set_var("JWT_SECRET", "production-secret");

        let config = TdxConfig::from_env();
        assert!(config.is_production());
        assert!(!config.is_dev_mode());
        assert_eq!(config.mode_description(), "Production (TEE enforced)");

        // Cleanup
        std::env::remove_var("TEE_ENFORCED");
        std::env::remove_var("DEV_MODE");
        std::env::remove_var("JWT_SECRET");
    }

    #[test]
    fn test_dev_mode() {
        std::env::set_var("TEE_ENFORCED", "false");
        std::env::set_var("DEV_MODE", "true");

        let config = TdxConfig::from_env();
        assert!(!config.is_production());
        assert!(config.is_dev_mode());
        assert_eq!(config.mode_description(), "Development (Mock attestation)");

        // Cleanup
        std::env::remove_var("TEE_ENFORCED");
        std::env::remove_var("DEV_MODE");
    }
}
