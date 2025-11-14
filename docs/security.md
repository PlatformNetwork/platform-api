# Security

## Overview

Platform API implements multiple layers of security to ensure safe challenge execution and data protection.

## Attestation

### TDX (Trust Domain Extensions)

- Hardware-based attestation for Intel TDX environments
- Quote verification for CVM integrity
- Secure communication channels

### SGX (Software Guard Extensions)

- Intel SGX enclave verification
- Memory encryption and isolation

### SEV-SNP (Secure Encrypted Virtualization)

- AMD SEV-SNP support
- Encrypted memory protection

## Encryption

### End-to-End Encryption

- ChaCha20-Poly1305 AEAD encryption
- X25519 key exchange
- HKDF key derivation

### Storage Encryption

- AES-256-GCM encryption for sensitive data
- Encryption keys stored in environment variables
- No hardcoded secrets in code

## Database Security

### Schema Isolation

- Each challenge has its own database schema
- Cross-schema access prevention
- Read-only permissions for challenge queries

### ORM Bridge Protocol

- Secure WebSocket communication
- Query validation and sanitization
- SQL injection prevention

## Network Security

### WebSocket Security

- TDX attestation required for connections
- Encrypted message transport
- Session token validation

### API Security

- Signature verification for public endpoints
- Hotkey validation
- Rate limiting (when configured)

## Best Practices

1. **Always use environment variables** for secrets
2. **Enable TEE_ENFORCED=true** in production
3. **Use strong encryption keys** (32+ bytes)
4. **Regular security audits** of challenge code
5. **Keep dependencies up to date**

## Security Checklist

- [ ] All secrets in environment variables
- [ ] TEE_ENFORCED=true in production
- [ ] Strong encryption keys configured
- [ ] Database access restricted
- [ ] TLS/HTTPS enabled for production
- [ ] Regular security updates applied

