// Unit tests for Attestation Service
// Uses mock TDX verifier (required, TDX needs hardware)

use platform_api_attestation::{AttestationService, AttestationConfig};
use platform_api_models::{AttestationRequest, AttestationType, AttestationStatus};
use std::sync::Arc;

// Helper to create test attestation service
fn create_test_attestation_service() -> AttestationService {
    // Use test configuration
    std::env::set_var("TEE_ENFORCED", "false");
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("JWT_SECRET", "test-secret");
    
    let config = platform_api_attestation::TdxConfig::from_env();
    AttestationService::new(&config).expect("Failed to create AttestationService")
}

#[tokio::test]
async fn test_attestation_service_creation() {
    let service = create_test_attestation_service();
    // Test that service can be created
    assert!(true);
}

#[tokio::test]
async fn test_attestation_request_missing_quote() {
    let service = create_test_attestation_service();
    
    // Set dev mode for testing
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("TEE_ENFORCED", "false");
    
    let request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: None, // Missing quote
        report: None,
        nonce: vec![],
        measurements: vec![],
        capabilities: vec![],
    };
    
    let response = service.verify_attestation(request).await
        .expect("Failed to verify attestation");
    
    // Should fail due to missing quote
    assert_eq!(response.status, AttestationStatus::Failed);
    assert!(response.error.is_some());
    assert!(response.error.unwrap().contains("Missing quote"));
    
    // Cleanup
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("TEE_ENFORCED");
}

#[tokio::test]
async fn test_attestation_request_short_nonce() {
    let service = create_test_attestation_service();
    
    // Set dev mode for testing
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("TEE_ENFORCED", "false");
    
    let request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(vec![0u8; 1024]), // Valid quote size
        report: None,
        nonce: vec![0u8; 10], // Too short (minimum 16 bytes)
        measurements: vec![],
        capabilities: vec![],
    };
    
    let response = service.verify_attestation(request).await
        .expect("Failed to verify attestation");
    
    // Should fail due to short nonce
    assert_eq!(response.status, AttestationStatus::Failed);
    assert!(response.error.is_some());
    assert!(response.error.unwrap().contains("too short") || response.error.unwrap().contains("Nonce"));
    
    // Cleanup
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("TEE_ENFORCED");
}

#[tokio::test]
async fn test_attestation_request_valid_structure() {
    let service = create_test_attestation_service();
    
    // Set dev mode for testing
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("TEE_ENFORCED", "false");
    
    let request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(vec![0u8; 1024]), // Valid quote size
        report: None,
        nonce: vec![0u8; 32], // Valid nonce size
        measurements: vec![vec![1, 2, 3, 4]],
        capabilities: vec![],
    };
    
    let response = service.verify_attestation(request).await
        .expect("Failed to verify attestation");
    
    // In dev mode with valid structure, should succeed
    assert_eq!(response.status, AttestationStatus::Verified);
    assert!(!response.session_token.is_empty());
    
    // Cleanup
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("TEE_ENFORCED");
}

