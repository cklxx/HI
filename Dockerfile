# syntax=docker/dockerfile:1.6

FROM rust:1.80-slim-bullseye AS chef
WORKDIR /workspace
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config && rm -rf /var/lib/apt/lists/*
COPY hi_telos/Cargo.toml hi_telos/Cargo.lock ./hi_telos/
RUN mkdir -p hi_telos/src && \
    echo 'fn main() {}' > hi_telos/src/main.rs && \
    cargo build --release --manifest-path hi_telos/Cargo.toml && \
    rm -rf hi_telos/src

FROM chef AS builder
COPY hi_telos/src ./hi_telos/src
RUN cargo build --release --manifest-path hi_telos/Cargo.toml

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
