# ------------------------------------------------------------------------------
#  Platform API Dockerfile (hardened)
#  – Builds production and local binaries
#  – Final runtime images run as non-root `platform` user (UID/GID 10001)
# ------------------------------------------------------------------------------

###############################################################################
# ---------- 1. Common build environment -------------------------------------
###############################################################################
ARG BASE_IMAGE=rust:1.90.0-slim
FROM ${BASE_IMAGE} AS base_builder

LABEL ai.platform.image.authors="platform@example.com" \
  ai.platform.image.vendor="Platform" \
  ai.platform.image.title="platform/api" \
  ai.platform.image.description="Platform API Server" \
  ai.platform.image.documentation="https://platform.network"

# Build prerequisites
ENV RUST_BACKTRACE=1
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN apt-get update && \
  apt-get install -y --no-install-recommends \
  build-essential pkg-config libssl-dev libgit2-dev ca-certificates git && \
  rm -rf /var/lib/apt/lists/*

# Copy dependency files and workspace structure first for better layer caching
# This allows Cargo to cache dependencies even when source code changes
COPY Cargo.toml Cargo.lock /build/
# Copy crates and bins directories (needed for workspace resolution)
COPY crates /build/crates
COPY bins /build/bins
WORKDIR /build

# Pre-fetch dependencies using BuildKit cache mount
# This layer will be cached and reused unless Cargo.toml/Cargo.lock changes
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo fetch --locked || true

# Copy entire repository (source code changes will invalidate this layer)
# This will overwrite crates/bins with actual source code
COPY . /build

###############################################################################
# ---------- 2. Production build stage ---------------------------------------
###############################################################################
FROM base_builder AS prod_builder

# Build the production binary with memory optimization
# Use BuildKit cache mounts for Cargo registry and target directory
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin platform-api-server --locked
RUN test -e /build/target/release/platform-api-server || (echo "ERROR: Binary not found!" && exit 1)

###############################################################################
# ---------- 3. Final production image (hardened) ---------------------------
###############################################################################
FROM debian:testing-slim AS platform-api

# Install adduser and runtime dependencies first
RUN apt-get update && apt-get install -y --no-install-recommends \
  adduser \
  ca-certificates libssl3 libgit2-1.9 && \
  rm -rf /var/lib/apt/lists/*

# ---- security hardening: create least-privilege user ----
RUN addgroup --system --gid 10001 platform && \
  adduser --system --uid 10001 --gid 10001 --home /home/platform --disabled-password platform

# Writable data directory
RUN mkdir -p /data && chown -R platform:platform /data

# Workdir for the non-root user
WORKDIR /home/platform

# Copy binary with correct ownership
COPY --from=prod_builder /build/target/release/platform-api-server /usr/local/bin/
RUN chown platform:platform /usr/local/bin/platform-api-server

EXPOSE 3000 9090

# Run as non-root user
USER platform

CMD ["/usr/local/bin/platform-api-server"]

###############################################################################
# ---------- 4. Local build stage -------------------------------------------
###############################################################################
FROM base_builder AS local_builder

# Build the workspace in release mode with memory optimization
# Use BuildKit cache mounts for Cargo registry and target directory
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin platform-api-server
RUN test -e /build/target/release/platform-api-server || (echo "ERROR: Binary not found!" && exit 1)

###############################################################################
# ---------- 5. Final local image (hardened) --------------------------------
###############################################################################
FROM debian:testing-slim AS platform-api-local

# Install adduser and runtime dependencies first
RUN apt-get update && apt-get install -y --no-install-recommends \
  adduser \
  ca-certificates libssl3 libgit2-1.9 && \
  rm -rf /var/lib/apt/lists/*

# Least-privilege user
RUN addgroup --system --gid 10001 platform && \
  adduser --system --uid 10001 --gid 10001 --home /home/platform --disabled-password platform

RUN mkdir -p /data && chown -R platform:platform /data
WORKDIR /home/platform

# Copy artifacts
COPY --from=local_builder /build/target/release/platform-api-server /usr/local/bin/
RUN chown platform:platform /usr/local/bin/platform-api-server

EXPOSE 3000 9090

# Run as non-root user
USER platform

CMD ["/usr/local/bin/platform-api-server"]
