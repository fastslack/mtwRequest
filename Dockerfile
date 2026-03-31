# =============================================================================
# mtwRequest Server — Multi-stage Docker Build
# =============================================================================
#
# Build:  docker build -t mtw-server .
# Run:    docker run -p 7741:7741 mtw-server
# Config: docker run -p 7741:7741 -v ./mtw.toml:/app/mtw.toml mtw-server
#
# Environment overrides:
#   MTW_HOST=0.0.0.0  MTW_PORT=7741  RUST_LOG=info,mtw=debug
#   RUST_BRIDGE_SOCKET=/tmp/mtw-rust.sock

# --- Builder stage ---
FROM rust:1.86-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/mtw-protocol/Cargo.toml crates/mtw-protocol/Cargo.toml
COPY crates/mtw-core/Cargo.toml crates/mtw-core/Cargo.toml
COPY crates/mtw-codec/Cargo.toml crates/mtw-codec/Cargo.toml
COPY crates/mtw-transport/Cargo.toml crates/mtw-transport/Cargo.toml
COPY crates/mtw-router/Cargo.toml crates/mtw-router/Cargo.toml
COPY crates/mtw-ai/Cargo.toml crates/mtw-ai/Cargo.toml
COPY crates/mtw-auth/Cargo.toml crates/mtw-auth/Cargo.toml
COPY crates/mtw-state/Cargo.toml crates/mtw-state/Cargo.toml
COPY crates/mtw-store/Cargo.toml crates/mtw-store/Cargo.toml
COPY crates/mtw-bridge/Cargo.toml crates/mtw-bridge/Cargo.toml
COPY crates/mtw-integrations/Cargo.toml crates/mtw-integrations/Cargo.toml
COPY crates/mtw-registry/Cargo.toml crates/mtw-registry/Cargo.toml
COPY crates/mtw-sdk/Cargo.toml crates/mtw-sdk/Cargo.toml
COPY crates/mtw-test/Cargo.toml crates/mtw-test/Cargo.toml
COPY crates/mtw-http/Cargo.toml crates/mtw-http/Cargo.toml
COPY crates/mtw-exchange/Cargo.toml crates/mtw-exchange/Cargo.toml
COPY crates/mtw-notify/Cargo.toml crates/mtw-notify/Cargo.toml
COPY crates/mtw-security/Cargo.toml crates/mtw-security/Cargo.toml
COPY crates/mtw-federation/Cargo.toml crates/mtw-federation/Cargo.toml
COPY crates/mtw-comms/Cargo.toml crates/mtw-comms/Cargo.toml
COPY crates/mtw-trading/Cargo.toml crates/mtw-trading/Cargo.toml
COPY crates/mtw-graph/Cargo.toml crates/mtw-graph/Cargo.toml
COPY crates/mtw-skills/Cargo.toml crates/mtw-skills/Cargo.toml
COPY crates/mtw-orchestrator/Cargo.toml crates/mtw-orchestrator/Cargo.toml
COPY crates/mtw-server/Cargo.toml crates/mtw-server/Cargo.toml
COPY examples/Cargo.toml examples/Cargo.toml

# Create dummy source files for dependency caching
RUN mkdir -p crates/mtw-protocol/src && echo "" > crates/mtw-protocol/src/lib.rs && \
    mkdir -p crates/mtw-core/src && echo "" > crates/mtw-core/src/lib.rs && \
    mkdir -p crates/mtw-codec/src && echo "" > crates/mtw-codec/src/lib.rs && \
    mkdir -p crates/mtw-transport/src && echo "" > crates/mtw-transport/src/lib.rs && \
    mkdir -p crates/mtw-router/src && echo "" > crates/mtw-router/src/lib.rs && \
    mkdir -p crates/mtw-ai/src && echo "" > crates/mtw-ai/src/lib.rs && \
    mkdir -p crates/mtw-auth/src && echo "" > crates/mtw-auth/src/lib.rs && \
    mkdir -p crates/mtw-state/src && echo "" > crates/mtw-state/src/lib.rs && \
    mkdir -p crates/mtw-store/src && echo "" > crates/mtw-store/src/lib.rs && \
    mkdir -p crates/mtw-bridge/src && echo "" > crates/mtw-bridge/src/lib.rs && \
    mkdir -p crates/mtw-integrations/src && echo "" > crates/mtw-integrations/src/lib.rs && \
    mkdir -p crates/mtw-registry/src && echo "" > crates/mtw-registry/src/lib.rs && \
    mkdir -p crates/mtw-sdk/src && echo "" > crates/mtw-sdk/src/lib.rs && \
    mkdir -p crates/mtw-test/src && echo "" > crates/mtw-test/src/lib.rs && \
    mkdir -p crates/mtw-http/src && echo "" > crates/mtw-http/src/lib.rs && \
    mkdir -p crates/mtw-exchange/src && echo "" > crates/mtw-exchange/src/lib.rs && \
    mkdir -p crates/mtw-notify/src && echo "" > crates/mtw-notify/src/lib.rs && \
    mkdir -p crates/mtw-security/src && echo "" > crates/mtw-security/src/lib.rs && \
    mkdir -p crates/mtw-federation/src && echo "" > crates/mtw-federation/src/lib.rs && \
    mkdir -p crates/mtw-comms/src && echo "" > crates/mtw-comms/src/lib.rs && \
    mkdir -p crates/mtw-trading/src && echo "" > crates/mtw-trading/src/lib.rs && \
    mkdir -p crates/mtw-graph/src && echo "" > crates/mtw-graph/src/lib.rs && \
    mkdir -p crates/mtw-skills/src && echo "" > crates/mtw-skills/src/lib.rs && \
    mkdir -p crates/mtw-orchestrator/src && echo "" > crates/mtw-orchestrator/src/lib.rs && \
    mkdir -p crates/mtw-server/src && echo "fn main() {}" > crates/mtw-server/src/main.rs && \
    mkdir -p examples && echo "fn main() {}" > examples/demo_server.rs && \
    echo "fn main() {}" > examples/demo_client.rs

# Build dependencies only (cached layer)
RUN cargo build --release -p mtw-server 2>/dev/null || true

# Now copy real source code
COPY crates/ crates/
COPY examples/ examples/

# Force cargo to detect source changes (COPY preserves host timestamps
# which may be older than the dependency-cache build artifacts)
RUN find crates/ examples/ -name "*.rs" -exec touch {} +

# Build the actual binary
RUN cargo build --release -p mtw-server

# --- Runtime stage ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates libssl3 curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /build/target/release/mtw-server /app/mtw-server

# Copy default config
COPY mtw.toml /app/mtw.toml

# Expose default port
EXPOSE 7741

# Environment defaults
ENV RUST_LOG=info,mtw=debug
ENV MTW_HOST=0.0.0.0
ENV MTW_PORT=7741
ENV RUST_BRIDGE_SOCKET=/tmp/mtw-rust.sock

CMD ["/app/mtw-server"]
