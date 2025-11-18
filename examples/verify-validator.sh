#!/bin/bash
# Example script to verify a platform validator using dstack-verifier

# Function to get attestation from validator
get_validator_attestation() {
    local validator_url=$1
    echo "Fetching attestation from validator at $validator_url..."
    
    # Get attestation from validator's /attestation endpoint
    curl -s "$validator_url/attestation" > attestation.json
    
    if [ $? -ne 0 ]; then
        echo "❌ Failed to fetch attestation"
        return 1
    fi
    
    echo "✅ Attestation fetched successfully"
    return 0
}

# Function to verify attestation with dstack-verifier
verify_attestation() {
    local verifier_url=${DSTACK_VERIFIER_URL:-http://localhost:8080}
    
    echo "Verifying attestation with dstack-verifier at $verifier_url..."
    
    # Send verification request
    response=$(curl -s -X POST \
        -H "Content-Type: application/json" \
        -d @attestation.json \
        "$verifier_url/verify")
    
    if [ $? -ne 0 ]; then
        echo "❌ Failed to verify attestation"
        return 1
    fi
    
    # Parse response
    is_valid=$(echo "$response" | jq -r '.is_valid')
    
    if [ "$is_valid" = "true" ]; then
        echo "✅ Platform verification PASSED"
        
        # Extract details
        tcb_status=$(echo "$response" | jq -r '.details.tcb_status')
        os_verified=$(echo "$response" | jq -r '.details.os_image_hash_verified')
        compose_hash=$(echo "$response" | jq -r '.details.app_info.compose_hash // "N/A"')
        kms_id=$(echo "$response" | jq -r '.details.app_info.key_provider.kms_id // "N/A"')
        
        echo ""
        echo "📊 Verification Details:"
        echo "  - TCB Status: $tcb_status"
        echo "  - OS Verified: $os_verified"
        echo "  - Compose Hash: $compose_hash"
        echo "  - KMS ID: $kms_id"
    else
        echo "❌ Platform verification FAILED"
        reason=$(echo "$response" | jq -r '.reason // "Unknown reason"')
        echo "  Reason: $reason"
        return 1
    fi
    
    # Save full response for inspection
    echo "$response" | jq '.' > verification_result.json
    echo ""
    echo "Full verification result saved to verification_result.json"
    
    return 0
}

# Main script
main() {
    local validator_url=${1:-http://localhost:8000}
    
    echo "🔍 Platform Validator Verification Tool"
    echo "======================================"
    echo ""
    
    # Check dependencies
    if ! command -v jq > /dev/null 2>&1; then
        echo "❌ jq is required. Please install it:"
        echo "   sudo apt-get install jq"
        exit 1
    fi
    
    # Get attestation
    if ! get_validator_attestation "$validator_url"; then
        exit 1
    fi
    
    echo ""
    
    # Verify attestation
    if ! verify_attestation; then
        exit 1
    fi
    
    echo ""
    echo "✅ Verification complete!"
}

# Run main function
main "$@"

