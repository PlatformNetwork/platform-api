use serde::{Deserialize, Serialize};

/// TDX Configuration with production/dev mode support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TdxConfig {
    /// Whether TEE attestation is enforced (production)
    pub tee_enforced: bool,
    /// Whether dev mode is enabled (mock attestation)
    pub dev_mode: bool,
    /// Session timeout in seconds
    pub session_timeout: u64,
    /// PCCS URL for collateral retrieval
    pub pccs_url: Option<String>,
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

        let session_timeout = std::env::var("SESSION_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(86400); // 24 hours default

        let pccs_url = std::env::var("PCCS_URL").ok();

        Self {
            tee_enforced,
            dev_mode,
            session_timeout,
            pccs_url,
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
        let tdx_simulation = std::env::var("TDX_SIMULATION_MODE")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true";

        match (self.tee_enforced, self.dev_mode, tdx_simulation) {
            (true, false, false) => "Production (TEE enforced)",
            (false, true, true) => "Development (Enhanced TDX simulation)",
            (false, true, false) => "Development (Mock attestation)",
            (true, true, _) => "Mixed (TEE with dev fallback)",
            (false, false, _) => "Development (TEE optional)",
            (_, _, true) => "Development (TDX simulation mode)",
        }
    }

    /// Check if TDX simulation mode is enabled
    pub fn is_tdx_simulation_mode(&self) -> bool {
        std::env::var("TDX_SIMULATION_MODE")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true"
    }

    /// Check if encryption should be enabled (always true)
    pub fn dev_mode_encryption_enabled(&self) -> bool {
        true // Encryption is always enabled
    }

    /// Get TDX mock level (full, basic, or error-injection)
    pub fn tdx_mock_level(&self) -> String {
        std::env::var("TDX_MOCK_LEVEL")
            .unwrap_or_else(|_| {
                if self.is_tdx_simulation_mode() {
                    "full".to_string()
                } else {
                    "basic".to_string()
                }
            })
            .to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_production_mode() {
        std::env::set_var("TEE_ENFORCED", "true");
        std::env::set_var("DEV_MODE", "false");

        let config = TdxConfig::from_env();
        assert!(config.is_production());
        assert!(!config.is_dev_mode());
        assert_eq!(config.mode_description(), "Production (TEE enforced)");

        // Cleanup
        std::env::remove_var("TEE_ENFORCED");
        std::env::remove_var("DEV_MODE");
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
