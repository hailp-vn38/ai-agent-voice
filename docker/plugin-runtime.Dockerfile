# syntax=docker/dockerfile:1
# Both installer and runner use this exact image. Plugin packages never build images.
FROM rust:1.98.1-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends cmake pkg-config && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo .cargo
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --locked --release -p voice-agent-server --bin plugin-runtime && \
    cp target/release/plugin-runtime /plugin-runtime

FROM python:3.12-slim-bookworm
COPY --from=ghcr.io/astral-sh/uv@sha256:4de5495181a281bc744845b9579acf7b221d6791f99bcc211b9ec13f417c2853 /uv /uvx /usr/local/bin/
COPY --from=builder /plugin-runtime /usr/local/bin/plugin-runtime
RUN mkdir -p /control /staging /installations /attempts \
    && chown -R 1000:1000 /control /staging /installations /attempts
USER 1000:1000
ENTRYPOINT ["/usr/local/bin/plugin-runtime"]
