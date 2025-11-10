// Integration tests for attestation flow
use platform_api_models::*;
use platform_api_attestation::{AttestationService, TdxConfig};
use chrono::{Utc, Duration};
use uuid::Uuid;
use serde_json::json;

#[tokio::test]
async fn test_complete_attestation_flow() {
    // Setup attestation service in dev mode
    std::env::set_var("TEE_ENFORCED", "false");
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("JWT_SECRET", "test-secret-key");
    
    let config = TdxConfig::from_env();
    assert!(config.is_dev_mode());
    assert_eq!(config.mode_description(), "Development (Mock attestation)");
    
    let service = AttestationService::new(&config)
        .expect("Failed to create attestation service");
    
    // 1. Create attestation challenge
    let challenge_response = service.create_challenge()
        .await
        .expect("Failed to create challenge");
    
    assert!(!challenge_response.nonce.is_empty());
    assert!(challenge_response.expires_at > Utc::now());
    
    // 2. Submit attestation request
    let attestation_request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(vec![0u8; 1024]), // Mock TDX quote
        report: None,
        nonce: challenge_response.nonce.clone(),
        measurements: vec![
            Measurement {
                name: "kernel".to_string(),
                value: "mock_kernel_hash_dev".to_string(),
                algorithm: "sha256".to_string(),
            },
            Measurement {
                name: "initrd".to_string(),
                value: "mock_initrd_hash_dev".to_string(),
                algorithm: "sha256".to_string(),
            },
        ],
        capabilities: vec!["tee".to_string(), "encrypted_storage".to_string()],
    };
    
    let attestation_response = service.verify_attestation(attestation_request)
        .await
        .expect("Failed to verify attestation");
    
    assert_eq!(attestation_response.status, AttestationStatus::Verified);
    assert!(!attestation_response.session_token.is_empty());
    assert!(attestation_response.expires_at > Utc::now());
    assert_eq!(attestation_response.verified_measurements.len(), 2);
    
    // 3. Get session details
    let session_id = extract_session_id_from_token(&attestation_response.session_token);
    let session = service.get_session(session_id)
        .await
        .expect("Failed to get session");
    
    assert_eq!(session.status, AttestationStatus::Verified);
    assert_eq!(session.attestation_type, AttestationType::Tdx);
    assert!(!session.validator_hotkey.is_empty());
    
    // 4. Verify token
    let claims = service.verify_token(&attestation_response.session_token)
        .expect("Failed to verify token");
    
    assert_eq!(claims["sub"], json!(session_id.to_string()));
    assert!(claims["exp"].as_u64().unwrap() > Utc::now().timestamp() as u64);
    
    // Cleanup
    std::env::remove_var("TEE_ENFORCED");
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("JWT_SECRET");
}

#[tokio::test]
async fn test_attestation_with_event_log() {
    std::env::set_var("TEE_ENFORCED", "false");
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("JWT_SECRET", "test-secret");
    
    let service = AttestationService::new(&TdxConfig::from_env())
        .expect("Failed to create attestation service");
    
    let event_log = json!({
        "compose_hash": "sha256:abcd1234",
        "environment_mode": "dev",
        "timestamp": Utc::now().to_rfc3339(),
        "metadata": {
            "version": "1.0",
            "challenge": "test-challenge"
        }
    });
    
    let request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(vec![0u8; 1024]),
        report: None,
        nonce: vec![0u8; 32],
        measurements: vec![
            Measurement {
                name: "compose_hash".to_string(),
                value: "sha256:abcd1234".to_string(),
                algorithm: "sha256".to_string(),
            }
        ],
        capabilities: vec![],
    };
    
    let response = service.verify_attestation_with_event_log(
        request,
        Some(&event_log.to_string())
    ).await.expect("Failed to verify with event log");
    
    assert_eq!(response.status, AttestationStatus::Verified);
    
    // Cleanup
    std::env::remove_var("TEE_ENFORCED");
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("JWT_SECRET");
}

#[tokio::test]
async fn test_attestation_production_mode_simulation() {
    // Simulate production mode
    std::env::set_var("TEE_ENFORCED", "true");
    std::env::set_var("DEV_MODE", "false");
    std::env::set_var("JWT_SECRET", "production-secret-key-minimum-32-chars");
    std::env::set_var("STORAGE_ENCRYPTION_KEY", "production-storage-key-32-bytes!");
    std::env::set_var("KBS_ENCRYPTION_KEY", "production-kbs-key-32-bytes-long");
    
    let config = TdxConfig::from_env();
    assert!(config.is_production());
    assert_eq!(config.mode_description(), "Production (TEE enforced)");
    
    let service = AttestationService::new(&config)
        .expect("Failed to create attestation service");
    
    // In production mode, invalid quote should fail
    let invalid_request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: None, // Missing quote
        report: None,
        nonce: vec![0u8; 32],
        measurements: vec![],
        capabilities: vec![],
    };
    
    let response = service.verify_attestation(invalid_request).await
        .expect("Should handle missing quote gracefully in API");
    
    assert_eq!(response.status, AttestationStatus::Failed);
    assert!(response.error.is_some());
    assert!(response.error.unwrap().contains("Missing quote"));
    
    // Cleanup
    std::env::remove_var("TEE_ENFORCED");
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("JWT_SECRET");
    std::env::remove_var("STORAGE_ENCRYPTION_KEY");
    std::env::remove_var("KBS_ENCRYPTION_KEY");
}

#[tokio::test]
async fn test_nonce_validation() {
    std::env::set_var("TEE_ENFORCED", "false");
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("JWT_SECRET", "test-secret");
    
    let service = AttestationService::new(&TdxConfig::from_env())
        .expect("Failed to create attestation service");
    
    // Test with too short nonce
    let short_nonce_request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(vec![0u8; 1024]),
        report: None,
        nonce: vec![0u8; 8], // Too short (min 16 bytes)
        measurements: vec![],
        capabilities: vec![],
    };
    
    let response = service.verify_attestation(short_nonce_request).await
        .expect("Should handle short nonce");
    
    assert_eq!(response.status, AttestationStatus::Failed);
    assert!(response.error.unwrap().contains("Nonce too short"));
    
    // Test with valid nonce
    let valid_nonce_request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(vec![0u8; 1024]),
        report: None,
        nonce: vec![0u8; 32], // Valid length
        measurements: vec![],
        capabilities: vec![],
    };
    
    let response = service.verify_attestation(valid_nonce_request).await
        .expect("Failed to verify with valid nonce");
    
    assert_eq!(response.status, AttestationStatus::Verified);
    
    // Cleanup
    std::env::remove_var("TEE_ENFORCED");
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("JWT_SECRET");
}

#[tokio::test]
async fn test_session_expiration() {
    std::env::set_var("TEE_ENFORCED", "false");
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("JWT_SECRET", "test-secret");
    std::env::set_var("SESSION_TIMEOUT", "1"); // 1 second timeout for testing
    
    let service = AttestationService::new(&TdxConfig::from_env())
        .expect("Failed to create attestation service");
    
    let request = AttestationRequest {
        attestation_type: AttestationType::Tdx,
        quote: Some(vec![0u8; 1024]),
        report: None,
        nonce: vec![0u8; 32],
        measurements: vec![],
        capabilities: vec![],
    };
    
    let response = service.verify_attestation(request).await
        .expect("Failed to verify attestation");
    
    let session_id = extract_session_id_from_token(&response.session_token);
    
    // Session should exist initially
    let session = service.get_session(session_id).await
        .expect("Failed to get session");
    assert_eq!(session.status, AttestationStatus::Verified);
    
    // Wait for expiration
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Session should be expired
    let expired_result = service.get_session(session_id).await;
    assert!(expired_result.is_err() || {
        if let Ok(s) = expired_result {
            s.expires_at < Utc::now()
        } else {
            false
        }
    });
    
    // Cleanup
    std::env::remove_var("TEE_ENFORCED");
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("JWT_SECRET");
    std::env::remove_var("SESSION_TIMEOUT");
}

#[tokio::test]
async fn test_concurrent_attestations() {
    use futures::future::join_all;
    
    std::env::set_var("TEE_ENFORCED", "false");
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("JWT_SECRET", "test-secret");
    
    let service = std::sync::Arc::new(
        AttestationService::new(&TdxConfig::from_env())
            .expect("Failed to create attestation service")
    );
    
    // Create multiple attestation requests concurrently
    let mut tasks = Vec::new();
    
    for i in 0..10 {
        let service_clone = service.clone();
        
        tasks.push(tokio::spawn(async move {
            let request = AttestationRequest {
                attestation_type: AttestationType::Tdx,
                quote: Some(vec![i as u8; 1024]),
                report: None,
                nonce: vec![i as u8; 32],
                measurements: vec![
                    Measurement {
                        name: "test".to_string(),
                        value: format!("value_{}", i),
                        algorithm: "sha256".to_string(),
                    }
                ],
                capabilities: vec![],
            };
            
            service_clone.verify_attestation(request).await
        }));
    }
    
    let results = join_all(tasks).await;
    
    // All should succeed
    for (i, result) in results.iter().enumerate() {
        let response = result.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(response.status, AttestationStatus::Verified, "Request {} failed", i);
        assert!(!response.session_token.is_empty());
    }
    
    // Cleanup
    std::env::remove_var("TEE_ENFORCED");
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("JWT_SECRET");
}

#[tokio::test]
async fn test_measurement_validation() {
    std::env::set_var("TEE_ENFORCED", "false");
    std::env::set_var("DEV_MODE", "true");
    std::env::set_var("JWT_SECRET", "test-secret");
    
    let service = AttestationService::new(&TdxConfig::from_env())
        .expect("Failed to create attestation service");
    
    // Test with various measurement scenarios
    let test_cases = vec![
        (
            vec![
                Measurement {
                    name: "kernel".to_string(),
                    value: "valid_hash".to_string(),
                    algorithm: "sha256".to_string(),
                }
            ],
            true,
            "Single valid measurement"
        ),
        (
            vec![
                Measurement {
                    name: "".to_string(), // Empty name
                    value: "hash".to_string(),
                    algorithm: "sha256".to_string(),
                }
            ],
            true, // Should still pass in dev mode
            "Empty measurement name"
        ),
        (
            vec![], // No measurements
            true, // Should pass in dev mode
            "No measurements"
        ),
        (
            vec![
                Measurement {
                    name: "kernel".to_string(),
                    value: "hash1".to_string(),
                    algorithm: "sha256".to_string(),
                },
                Measurement {
                    name: "initrd".to_string(),
                    value: "hash2".to_string(),
                    algorithm: "sha256".to_string(),
                },
                Measurement {
                    name: "cmdline".to_string(),
                    value: "hash3".to_string(),
                    algorithm: "sha384".to_string(),
                }
            ],
            true,
            "Multiple measurements with different algorithms"
        ),
    ];
    
    for (measurements, should_pass, description) in test_cases {
        let request = AttestationRequest {
            attestation_type: AttestationType::Tdx,
            quote: Some(vec![0u8; 1024]),
            report: None,
            nonce: vec![0u8; 32],
            measurements: measurements.clone(),
            capabilities: vec![],
        };
        
        let result = service.verify_attestation(request).await;
        
        if should_pass {
            let response = result.expect(&format!("Failed: {}", description));
            assert_eq!(
                response.status,
                AttestationStatus::Verified,
                "Test case '{}' should have passed",
                description
            );
            assert_eq!(response.verified_measurements, measurements);
        } else {
            assert!(
                result.is_err() || result.unwrap().status == AttestationStatus::Failed,
                "Test case '{}' should have failed",
                description
            );
        }
    }
    
    // Cleanup
    std::env::remove_var("TEE_ENFORCED");
    std::env::remove_var("DEV_MODE");
    std::env::remove_var("JWT_SECRET");
}

// Helper functions

fn extract_session_id_from_token(token: &str) -> Uuid {
    // In real implementation, decode JWT to get session ID
    // For testing, we'll parse from the token structure
    use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
    
    let decoding_key = DecodingKey::from_secret(b"test-secret");
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&["platform-executor"]);
    
    let token_data = decode::<serde_json::Value>(
        token,
        &decoding_key,
        &validation
    ).expect("Failed to decode token");
    
    Uuid::parse_str(
        token_data.claims["sub"].as_str().unwrap()
    ).expect("Failed to parse session ID")
}
