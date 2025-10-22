# syntax=docker/dockerfile:1.6

FROM rust:1.80-slim-bullseye AS chef
WORKDIR /workspace
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates/hi_telos/Cargo.toml crates/hi_telos/
RUN mkdir -p crates/hi_telos/src && \
    echo 'fn main() {}' > crates/hi_telos/src/main.rs && \
    cargo build --release --package hi_telos && \
    rm -rf crates

FROM chef AS builder
COPY crates/hi_telos/src ./crates/hi_telos/src
COPY crates/hi_telos/tests ./crates/hi_telos/tests
RUN cargo build --release --package hi_telos

FROM debian:bullseye-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /workspace/target/release/hi_telos /usr/local/bin/hi_telos
COPY config ./config
ENV HI_APP_ROOT=/app \
    HI_SERVER_BIND=0.0.0.0:8080
VOLUME ["/app/data"]
EXPOSE 8080
ENTRYPOINT ["hi_telos"]
