# Architecture

## Overview

Platform API is the central orchestrator for Platform Network, managing challenge deployment, job distribution, validator coordination, and secure attestation verification.

## Core Components

### API Server (`bins/platform-api-server`)

Main HTTP server built with Axum that handles:
- REST API endpoints for challenge and job management
- WebSocket connections for real-time communication
- Request routing and middleware
- Metrics collection (Prometheus)

### API Crate (`crates/api`)

Core API logic including:
- Challenge runner and lifecycle management
- CVM (Confidential Virtual Machine) manager
- WebSocket handler for challenge connections
- ORM bridge for secure database access
- Challenge credentials and attestation

### Storage (`crates/storage`)

Database abstraction layer:
- PostgreSQL backend implementation
- Encryption at rest
- Challenge metadata management
- Job and result storage

### Attestation (`crates/attestation`)

Attestation verification:
- TDX (Trust Domain Extensions) verification
- SGX (Software Guard Extensions) support
- SEV-SNP (Secure Encrypted Virtualization) support
- JWT-based session management

### Key Broker Service (`crates/kbs`)

Secure key management:
- Encryption key derivation (HKDF)
- Session-based key management
- Secure key storage

### Builder (`crates/builder`)

Challenge build system:
- Docker image building
- Git repository management
- Build cache management

### Scheduler (`crates/scheduler`)

Job scheduling:
- Job queue management
- Validator assignment
- Result collection

### Autoscaler (`crates/autoscaler`)

Automatic scaling of challenge instances:
- Monitor challenge load
- Scale CVM instances up/down
- Resource optimization
- Cost management

### Models (`crates/models`)

Shared data models and schemas:
- Challenge specifications
- Job definitions
- Validator status
- Result structures

## Architecture Diagram

```
┌─────────────────┐
│  Platform API   │
│  (Central API)  │
└────────┬────────┘
         │
    ┌────┴────┐
    │         │
┌───▼───┐ ┌──▼────┐
│Validat│ │Challenge│
│ors    │ │CVMs     │
└───────┘ └────────┘
```

## Data Flow

1. **Challenge Deployment**: Challenges are deployed via API and registered in the system
2. **Job Distribution**: Jobs are queued and distributed to available validators
3. **Execution**: Validators execute jobs in TDX-secured CVMs
4. **Result Collection**: Results are collected and stored securely
5. **Attestation**: All communications use TDX attestation for security

## Security Features

- TDX/SGX/SEV-SNP attestation verification
- End-to-end encryption (ChaCha20-Poly1305 with X25519 key exchange)
- Schema-based database isolation
- Secure WebSocket connections
- JWT-based authentication

