# Build stage
FROM rust:latest as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY dashboard.html ./

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:trixie-slim

# Install dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/llm_cache_proxy .

EXPOSE 3000

CMD ["./llm_cache_proxy"]