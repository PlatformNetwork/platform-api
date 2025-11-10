# TDX Attestation Configuration Guide

This document describes how to configure TDX attestation for production environments with development mode fallback.

## Overview

Platform API and Platform Validator support TDX attestation for secure execution environments. The system can operate in three modes:

1. **Production Mode**: Full TDX attestation required (TEE_ENFORCED=true)
2. **Development Mode**: Mock attestation with structure validation (DEV_MODE=true)
3. **Mixed Mode**: Production with dev fallback (both flags configured)

## Environment Variables

### Platform API Configuration

```bash
# Production Mode (TDX Required)
TEE_ENFORCED=true
DEV_MODE=false

# Development Mode (Mock Attestation)
TEE_ENFORCED=false
DEV_MODE=true

# Production with Dev Fallback
TEE_ENFORCED=true
DEV_MODE=false
# But handle errors gracefully in code
```

### Platform Validator Configuration

```bash
# Enable TDX attestation
ENABLE_ATTESTATION=true

# Dev mode flag
DEV_MODE=false

# TDX-specific configuration
DSTACK_GUEST_TDX_PATH=/dev/tdx-guest  # Path to TDX device
```

## Attestation Flow

### 1. Production Mode (TEE_ENFORCED=true, DEV_MODE=false)

- Full TDX quote verification required
- No bypassing allowed
- Failures result in connection rejection
- Used for mainnet deployments

### 2. Development Mode (TEE_ENFORCED=false OR DEV_MODE=true)

- Mock attestation with structure validation
- Validates request format and nonce binding
- Returns mock measurements
- Used for local development and testing

### 3. Error Handling

In production, if TDX is not available:
```rust
// Platform API will fail fast with clear error
if tee_enforced && !dev_mode {
    return Err(anyhow::anyhow!(
        "Security error: Cannot get compose_hash from TDX attestation in production mode"
    ));
}
```

## Security Configuration

### JWT Secret

```bash
# Production: Use strong secret
JWT_SECRET=<strong-random-secret>

# Development: Can disable JWT
JWT_SECRET=disabled-no-jwt
```

### Storage Encryption

```bash
# Production: Required
STORAGE_ENCRYPTION_KEY=<32-byte-key>

# Development: Optional
STORAGE_ENCRYPTION_KEY=dev-key-not-for-production
```

### KBS Encryption

```bash
# Production: Required for key broker service
KBS_ENCRYPTION_KEY=<32-byte-key>
```

## Production Checklist

### Platform API

- [ ] Set `TEE_ENFORCED=true`
- [ ] Set `DEV_MODE=false`
- [ ] Configure strong `JWT_SECRET`
- [ ] Set `STORAGE_ENCRYPTION_KEY` (32 bytes)
- [ ] Set `KBS_ENCRYPTION_KEY` (32 bytes)
- [ ] Verify TDX is available on the host

### Platform Validator

- [ ] Set `ENABLE_ATTESTATION=true`
- [ ] Set `DEV_MODE=false`
- [ ] Ensure `/dev/tdx-guest` is available
- [ ] Configure `DSTACK_GUEST_TDX_PATH` if non-standard

## Testing Configuration

### Local Development

```bash
# Platform API
TEE_ENFORCED=false
DEV_MODE=true
JWT_SECRET=disabled-no-jwt
STORAGE_ENCRYPTION_KEY=dev-key
KBS_ENCRYPTION_KEY=dev-key

# Platform Validator
ENABLE_ATTESTATION=false
DEV_MODE=true
```

### Staging/Testing with TDX

```bash
# Platform API
TEE_ENFORCED=true
DEV_MODE=false
JWT_SECRET=staging-secret
STORAGE_ENCRYPTION_KEY=<staging-key>
KBS_ENCRYPTION_KEY=<staging-key>

# Platform Validator
ENABLE_ATTESTATION=true
DEV_MODE=false
```

## Monitoring

### Key Metrics to Monitor

1. **Attestation Success Rate**
   - Track successful vs failed attestations
   - Alert on sudden drops

2. **Quote Generation Time**
   - Monitor TDX quote generation latency
   - Alert on performance degradation

3. **Session Expiry**
   - Track active attestation sessions
   - Monitor session renewal patterns

### Logs to Watch

```bash
# Success indicators
INFO: "Verifying attestation request with TDX verifier"
INFO: "TDX attestation verified successfully"

# Warning indicators
WARN: "DEV MODE: Using mock TDX verification"
WARN: "TEE_ENFORCED=false - not recommended for production"

# Error indicators
ERROR: "TDX attestation verification failed"
ERROR: "Cannot get compose_hash from TDX attestation"
ERROR: "Security error: app_id missing from verification result"
```

## Troubleshooting

### Common Issues

1. **"Cannot get compose_hash from TDX attestation"**
   - Ensure TDX is available on the host
   - Check `/dev/tdx-guest` exists
   - Verify dstack is properly configured

2. **"TDX attestation verification failed"**
   - Check quote format
   - Verify nonce binding
   - Check event log structure

3. **"JWT authentication is disabled"**
   - Set proper JWT_SECRET in production
   - Remove "disabled-no-jwt" value

## Security Best Practices

1. **Never use DEV_MODE=true in production**
2. **Always set TEE_ENFORCED=true for mainnet**
3. **Use strong, unique secrets for each environment**
4. **Monitor attestation logs for anomalies**
5. **Implement rate limiting on attestation endpoints**
6. **Regular security audits of attestation flow**

## Migration Guide

### From Dev to Production

1. Generate production secrets:
   ```bash
   # Generate JWT secret
   openssl rand -hex 32
   
   # Generate encryption keys
   openssl rand -hex 32  # For STORAGE_ENCRYPTION_KEY
   openssl rand -hex 32  # For KBS_ENCRYPTION_KEY
   ```

2. Update environment variables
3. Test attestation flow in staging
4. Deploy with monitoring enabled
5. Verify attestation metrics post-deployment
