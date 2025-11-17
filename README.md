<div align="center">

<pre>
█▀█ █░░ ▄▀█ ▀█▀ █▀▀ █▀█ █▀█ █▀▄▀█   ▄▀█ █▀█ █
█▀▀ █▄▄ █▀█ ░█░ █▀░ █▄█ █▀▄ █░▀░█   █▀█ █▀▀ █
</pre>

<a name="readme-top"></a>

A secure, high-performance API orchestrator for Platform Network that manages<br/>challenge deployment, job distribution, validator coordination, and TDX/SGX/SEV-SNP attestation,<br/>built with [Rust](https://www.rust-lang.org/) and [Axum](https://github.com/tokio-rs/axum).

[![Rust version][rust_version_img]][rust_url]
[![License][repo_license_img]][repo_license_url]

**↗️ The official Platform API documentation ↗️**

[Getting Started](docs/getting-started.md) · [Architecture](docs/architecture.md) · [Security](docs/security.md) · [API Reference](docs/api-reference.md)

</div>

> [!CAUTION]
> Platform API is currently in early development. Some features may be incomplete, APIs may change, and potential security vulnerabilities may exist. The team is actively testing to ensure everything is properly implemented and stable. Not ready for production use.



## Related Projects

Platform Network consists of several interconnected components that work together to provide a secure, decentralized challenge evaluation system:

| Project | Repository | Description |
|---------|-----------|-------------|
| **Platform Validator** | [PlatformNetwork/platform](https://github.com/PlatformNetwork/platform) | Secure, high-performance validator built in Rust that executes challenges in TDX-secured VMs via dstack VMM. Manages job execution, challenge lifecycle, CVM provisioning, resource quota allocation, and result submission. Provides WebSocket connectivity to Platform API and challenge CVMs. |
| **Challenge SDK** | [PlatformNetwork/challenge](https://github.com/PlatformNetwork/challenge) | Modern Python SDK for building verifiable challenges on Platform Network. Provides decorator-based lifecycle management, encrypted WebSocket communication with TDX attestation, automatic database migrations, custom weights calculation, and public API endpoints. |


## Features

- **Challenge Management**: Deploy, configure, and monitor challenges across the network
- **Job Distribution**: Intelligent job queuing and distribution to validators
- **Validator Coordination**: Manage validator connections, challenge status, and resource allocation
- **Attestation Verification**: TDX/SGX/SEV-SNP attestation verification for secure challenge execution
- **ORM Bridge**: Secure database access bridge for challenge database operations
- **Token Emission**: Scheduling and management of token emissions
- **Public Endpoint Proxying**: Proxy requests to challenge public APIs with signature verification
- **WebSocket Support**: Real-time communication with validators and challenges

<div align="right">

[↗ Back to top](#readme-top)

</div>

## Quick Start

> [!NOTE]
> Platform API requires Rust 1.70 or higher and a PostgreSQL database.

### Docker Compose - Development

```bash
docker-compose up --build
```

To run in background:
```bash
docker-compose up -d --build
```

To view logs:
```bash
docker-compose logs -f
```

To stop:
```bash
docker-compose down
```

### Docker Compose - Production (HTTPS)

The production setup uses nginx as a reverse proxy with HTTPS support for `api.platform.network`.

**1. Generate or obtain SSL certificates:**

For testing (self-signed):
```bash
./generate-test-certs.sh
```

For production (Let's Encrypt):
```bash
# See certs/README.md for detailed instructions
sudo certbot certonly --standalone -d api.platform.network
sudo cp /etc/letsencrypt/live/api.platform.network/*.pem ./certs/
```

**2. Configure environment variables:**

Create a `.env` file with required variables:
```bash
DATABASE_URL=postgresql://user:password@host:5432/platform
STORAGE_ENCRYPTION_KEY=your-encryption-key
KBS_ENCRYPTION_KEY=your-kbs-encryption-key
```

**3. Start the production stack:**

```bash
docker-compose -f docker-compose.production.yml up -d
```

**4. Verify HTTPS is working:**

```bash
curl -I https://api.platform.network/health
```

The production setup includes:
- ✅ HTTPS with TLS 1.2/1.3
- ✅ Automatic HTTP to HTTPS redirect
- ✅ Security headers (HSTS, CSP, etc.)
- ✅ Rate limiting
- ✅ WebSocket support
- ✅ CORS configuration
- ✅ Gzip compression

### Docker Direct

Build and run production:
```bash
docker build -t platform-api:prod --target platform-api .
docker run -p 3000:3000 -p 9090:9090 platform-api:prod
```

Build and run local development:
```bash
docker build -t platform-api:local --target platform-api-local .
docker run -p 3000:3000 -p 9090:9090 platform-api:local
```

### Local with Cargo

```bash
cargo run --release --bin platform-api-server
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `RUST_LOG` | Log level | `info` |
| `ENVIRONMENT_MODE` | Environment mode (`dev` or `prod`) | `dev` |
| `TEE_ENFORCED` | Enable TEE verification | `false` |
| `SERVER_HOST` | Server bind address | `0.0.0.0` |
| `SERVER_PORT` | HTTP server port | `3000` |
| `PUBLIC_URL` | Public URL for API (used in production with HTTPS) | - |
| `METRICS_PORT` | Prometheus metrics port | `9090` |
| `DATABASE_URL` | PostgreSQL connection string | `postgresql://localhost/platform` |
| `STORAGE_BACKEND` | Storage backend type | `postgres` |
| `STORAGE_ENCRYPTION_KEY` | Encryption key for storage (required in production) | - |
| `JWT_SECRET` | JWT signing secret (required in production) | - |
| `KBS_ENCRYPTION_KEY` | Key Broker Service encryption key (required in production) | - |
| `DEV_MODE` | Enable development mode features | `true` |

### Ports

#### Development
- **3000**: HTTP API server
- **9090**: Prometheus metrics endpoint

#### Production (with nginx reverse proxy)
- **80**: HTTP (redirects to HTTPS)
- **443**: HTTPS API server (proxied to internal port 3000)
- **9090**: Prometheus metrics (internal only)

## Documentation

For complete documentation, see:

- **[Getting Started](docs/getting-started.md)** - Installation, prerequisites, and quick start guide
- **[Architecture](docs/architecture.md)** - System architecture and component overview
- **[Security](docs/security.md)** - Security architecture and attestation
- **[API Reference](docs/api-reference.md)** - Complete API documentation

## License

```
Copyright 2025 Cortex Foundation

Licensed under the MIT License.

See LICENSE file for details.
```

<div align="right">

[↗ Back to top](#readme-top)

</div>

---

<div align="center">

**[Back to top](#readme-top)**

Made with love by the Cortex Foundation

</div>

<!-- Rust links -->

[rust_url]: https://www.rust-lang.org/
[rust_version_img]: https://img.shields.io/badge/Rust-1.70+-blue?style=for-the-badge&logo=rust

<!-- Repository links -->

[repo_license_url]: https://github.com/PlatformNetwork/platform-api/blob/main/LICENSE
[repo_license_img]: https://img.shields.io/badge/license-MIT-blue?style=for-the-badge&logo=none
