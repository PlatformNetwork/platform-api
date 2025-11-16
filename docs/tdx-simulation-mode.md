# TDX Simulation Mode for Development

## Overview

TDX Simulation Mode enables full testing of the TDX attestation infrastructure in development environments without requiring TDX hardware. This mode simulates the complete security pipeline including quote generation, key exchange, and encrypted communications.

## Features

- **Mock TDX Quote Generation**: Generates structurally valid TDX quotes with proper report_data placement
- **Nonce Binding**: Verifies nonce binding in quotes (SHA256 hash embedded in report_data)
- **Realistic Event Logs**: Generates event logs with compose_hash, app_id, and instance_id
- **Encrypted Communications**: Always uses ChaCha20Poly1305 encryption for WebSocket messages
- **Key Exchange**: Performs X25519 key exchange and HKDF key derivation
- **Measurement Extraction**: Extracts and validates measurements from mock quotes

## Configuration

### Environment Variables

#### `TDX_SIMULATION_MODE`
Enable full TDX simulation mode. When set to `true`:
- Uses enhanced mock TDX verification with crypto operations
- Enables encrypted WebSocket communications
- Generates realistic quotes and event logs
- Validates nonce binding

**Default**: `false`

#### `TDX_MOCK_LEVEL`
Control the fidelity of TDX simulation:
- `full`: Complete simulation with all features (default when TDX_SIMULATION_MODE=true)
- `basic`: Basic structural validation only
- `error-injection`: Full simulation with error injection capabilities (future)

**Default**: `basic`

## Usage

### Basic Setup

Enable TDX simulation mode in your docker-compose.dev.yml:

```yaml
environment:
  - TDX_SIMULATION_MODE=true
  - TDX_MOCK_LEVEL=full
```

Or set environment variables:

```bash
export TDX_SIMULATION_MODE=true
export TDX_MOCK_LEVEL=full
```

### Platform-API Configuration

In `platform-api`, the simulation mode is automatically detected and used when:
- `TEE_ENFORCED=false` OR `DEV_MODE=true` OR `TDX_SIMULATION_MODE=true`

The attestation service will:
1. Generate mock quotes with proper structure
2. Verify nonce binding
3. Extract app_id, instance_id, and compose_hash from event logs
4. Always use encrypted communications

### Platform Validator Configuration

The validator automatically generates enhanced mock quotes when:
- `VALIDATOR_MOCK_VMM=true` OR `ENVIRONMENT_MODE=dev`

Enhanced mock quotes include:
- Proper report_data placement at multiple offsets (368, 568, 576)
- Realistic RTMRs generated using SHA384
- Event logs with app_id, instance_id, and compose_hash
- Proper event digests

### Challenge SDK Configuration

The challenge SDK checks for:
- `SDK_DEV_MODE`: Basic dev mode (plain text)
- `TDX_SIMULATION_MODE`: Enhanced dev mode with encryption

When encryption is enabled, the SDK:
- Waits for `attestation_ok` message
- Derives AEAD keys using HKDF
- Uses `AeadSession` for encrypted communications

## Benefits

1. **End-to-End Security Testing**: Test the complete attestation flow without TDX hardware
2. **Encryption Testing**: Validate encryption/decryption logic in dev environment
3. **Key Exchange Testing**: Test X25519 key exchange and HKDF derivation
4. **Error Handling**: Test error scenarios and edge cases
5. **Performance Testing**: Measure crypto overhead and optimize message handling

## Architecture

### Mock TDX Quote Structure

Mock quotes are 1024 bytes with:
- Random data for most of the quote
- Report data (SHA256(nonce)) embedded at offsets 368, 568, or 576
- Proper structure matching real TDX quotes

### Event Log Format

Event logs are JSON arrays with events:
```json
[
  {
    "imr": 3,
    "event_type": 1,
    "digest": "<sha384_digest>",
    "event": "app-id",
    "event_payload": "<app_id>"
  },
  {
    "imr": 3,
    "event_type": 2,
    "digest": "<sha384_digest>",
    "event": "instance-id",
    "event_payload": "<instance_id>"
  },
  {
    "imr": 3,
    "event_type": 3,
    "digest": "<sha384_digest>",
    "event": "compose-hash",
    "event_payload": "<compose_hash>"
  }
]
```

### RTMR Generation

RTMRs are generated using SHA384:
- RTMR0: Initial state (all zeros)
- RTMR1: SHA384(RTMR0 || "kernel")
- RTMR2: SHA384(RTMR1 || "initrd")
- RTMR3: SHA384(RTMR2 || event_log_digests)

## Testing

### Verify Simulation Mode

Check logs for:
```
DEV MODE: Using enhanced mock TDX verification (structure validation and crypto operations)
DEV MODE (SIMULATION): Requesting migrations via encrypted WebSocket
Mock TDX verification completed
```

### Verify Encryption

Check that messages are encrypted:
- Look for `EncryptedEnvelope` structures in WebSocket messages
- Verify nonce and ciphertext fields are present
- Check that plain text messages are NOT sent when encryption is enabled

## Limitations

1. **No Real TDX Hardware**: Quotes are not cryptographically signed by Intel
2. **No Real Attestation**: Cannot verify against Intel's attestation service
3. **Mock Measurements**: MRs are generated, not extracted from real hardware
4. **Development Only**: Should never be used in production

## Future Enhancements

- Error injection capabilities (`TDX_MOCK_LEVEL=error-injection`)
- Performance profiling and metrics
- Integration test suite
- Advanced attack scenario simulation

