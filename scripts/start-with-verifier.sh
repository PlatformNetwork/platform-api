#!/bin/bash
# Script to start platform-api with dstack-verifier

echo "🚀 Starting Platform-API with DStack Verifier"

# Check if dstack-verifier is running
if ! curl -s http://localhost:8080/health > /dev/null 2>&1; then
    echo "⚠️  DStack verifier not running. Starting it..."
    
    # Try Docker first
    if command -v docker > /dev/null 2>&1; then
        echo "Starting dstack-verifier with Docker..."
        docker run -d \
            -p 8080:8080 \
            --name dstack-verifier \
            --rm \
            dstacktee/dstack-verifier:latest
        
        # Wait for verifier to start
        sleep 3
    else
        echo "❌ Docker not found. Please start dstack-verifier manually:"
        echo "   cd /home/ubuntu/repo/all/Infinity/dstack/verifier"
        echo "   cargo run --bin dstack-verifier"
        exit 1
    fi
fi

# Check if verifier is healthy
if curl -s http://localhost:8080/health > /dev/null 2>&1; then
    echo "✅ DStack verifier is running"
else
    echo "❌ DStack verifier failed to start"
    exit 1
fi

# Export environment variable
export DSTACK_VERIFIER_URL=http://localhost:8080
echo "✅ DSTACK_VERIFIER_URL=$DSTACK_VERIFIER_URL"

# Other required environment variables
export PCCS_URL="${PCCS_URL:-https://pccs.bittensor.com/sgx/certification/v4/}"
echo "✅ PCCS_URL=$PCCS_URL"

# Start platform-api
echo "🎯 Starting platform-api with full platform verification enabled..."
cd /home/ubuntu/repo/all/Infinity/platform-api
cargo run --bin platform-api-server

# Cleanup on exit
trap "docker stop dstack-verifier 2>/dev/null || true" EXIT

