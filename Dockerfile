# ------------------------------------------------------------------------------
#  Platform API Dockerfile - ALTERNATIVE OPTIMIZED VERSION
#  – Uses BuildKit cache mounts to preserve target/ directory between builds
#  – No need for cargo-chef, simpler approach
# ------------------------------------------------------------------------------

FROM rust:1.90.0-slim AS builder

# Install build dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev libgit2-dev ca-certificates git && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy bittensor-rs first (needed as a local dependency)
# Copy to /build/bittensor-rs, then create symlink at ../bittensor-rs to match Cargo.toml path
COPY bittensor-rs ./bittensor-rs
RUN ln -s /build/bittensor-rs /bittensor-rs

# Copy only Cargo files first
COPY platform-api/Cargo.toml platform-api/Cargo.lock ./

# Create directory structure and copy Cargo.toml files
RUN mkdir -p crates/api crates/storage crates/models \
             crates/scheduler crates/builder crates/attestation \
             crates/kbs crates/autoscaler \
             bins/platform-api-server

# Copy each Cargo.toml individually (COPY doesn't work well with wildcards)
COPY platform-api/crates/api/Cargo.toml crates/api/
COPY platform-api/crates/storage/Cargo.toml crates/storage/
COPY platform-api/crates/models/Cargo.toml crates/models/
COPY platform-api/crates/scheduler/Cargo.toml crates/scheduler/
COPY platform-api/crates/builder/Cargo.toml crates/builder/
COPY platform-api/crates/attestation/Cargo.toml crates/attestation/
COPY platform-api/crates/kbs/Cargo.toml crates/kbs/
COPY platform-api/crates/autoscaler/Cargo.toml crates/autoscaler/
COPY platform-api/bins/platform-api-server/Cargo.toml bins/platform-api-server/

# Create dummy source files to build dependencies
RUN mkdir -p bins/platform-api-server/src && \
    echo "fn main() {}" > bins/platform-api-server/src/main.rs && \
    for crate in api storage models scheduler builder attestation kbs autoscaler; do \
        mkdir -p crates/$crate/src && \
        echo "fn main() {}" > crates/$crate/src/lib.rs; \
    done

# Build dependencies with cache mount
# This step is cached as long as Cargo.toml files don't change
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin platform-api-server

# Now copy the actual source code
COPY platform-api/ .

# Touch all source files to ensure they're newer than the cached build
RUN find . -name "*.rs" -exec touch {} \;

# Build the actual application
# The cache mount preserves compiled dependencies
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin platform-api-server && \
    # IMPORTANT: Copy the binary out before the mount is unmounted
    cp /build/target/release/platform-api-server /platform-api-server

###############################################################################
# Runtime stage (minimal image)
###############################################################################
FROM debian:testing-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 libgit2-1.9 curl && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd --system --gid 10001 platform && \
    useradd --system --uid 10001 --gid 10001 --home /home/platform --create-home platform

RUN mkdir -p /data && chown -R platform:platform /data
WORKDIR /home/platform

# Copy binary
COPY --from=builder /platform-api-server /usr/local/bin/platform-api-server
RUN chmod +x /usr/local/bin/platform-api-server && \
    chown platform:platform /usr/local/bin/platform-api-server

EXPOSE 15000 10090

USER platform

HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD ["sh", "-c", "curl -f http://localhost:${SERVER_PORT:-15000}/health || exit 1"]

CMD ["/usr/local/bin/platform-api-server"]

FROM runtime AS platform-api-local
FROM runtime AS platform-api
