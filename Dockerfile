# Stage 1: Build release binary
FROM rust:1-slim AS builder

WORKDIR /usr/src/runvane

# Install protobuf compiler and build dependencies for tonic-build
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace source files
COPY Cargo.toml Cargo.lock build.rs ./
COPY proto ./proto
COPY src ./src

# Compile release binary
RUN cargo build --release --bin runvane

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim AS runtime

# Install CA certificates and runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

# Create unprivileged system user and state directories
RUN groupadd -g 1000 runvane && \
    useradd -u 1000 -g runvane -s /bin/false -m runvane && \
    mkdir -p /var/lib/runvane /etc/runvane && \
    chown -R runvane:runvane /var/lib/runvane /etc/runvane

# Copy binary from builder
COPY --from=builder /usr/src/runvane/target/release/runvane /usr/local/bin/runvane

# Copy reference configuration
COPY config/runvane.example.toml /etc/runvane/runvane.toml

USER runvane
WORKDIR /var/lib/runvane

# Expose HTTP API & Dashboard (8080) and gRPC Control Plane (9090)
EXPOSE 8080 9090

# Healthcheck against HTTP health endpoint
HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/runvane"]
CMD ["serve", "--config", "/etc/runvane/runvane.toml"]
