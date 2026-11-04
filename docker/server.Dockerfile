# syntax=docker/dockerfile:1
FROM rust:1.98.1-bookworm AS source
RUN apt-get update && apt-get install -y --no-install-recommends cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
ENV CARGO_TARGET_DIR=/mnt/storage/ai-agent-voice/target
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY prompts prompts
COPY assets assets
COPY config.example.toml ./
COPY docker docker

FROM source AS test
RUN rustup component add rustfmt clippy
COPY scripts scripts
CMD ["bash", "-c", "cargo fmt --check && cargo clippy --locked --all-targets --all-features -- -D warnings && cargo test --locked --workspace && cargo test --locked --workspace --features qualification-providers"]

FROM source AS release
RUN --mount=type=cache,id=vela-cargo,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=vela-target,target=/mnt/storage/ai-agent-voice/target,sharing=locked \
    cargo build --locked --release -p vela-server --bin vela-server \
    && cp /mnt/storage/ai-agent-voice/target/release/vela-server /vela-server

FROM source AS qualification-build
RUN --mount=type=cache,id=vela-cargo,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=vela-target,target=/mnt/storage/ai-agent-voice/target,sharing=locked \
    cargo build --locked -p vela-server --bin vela-server --features qualification-providers \
    && cp /mnt/storage/ai-agent-voice/target/debug/vela-server /vela-server

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl libstdc++6 libgomp1 xz-utils \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY assets/enrollment assets/enrollment
ENV VELA_CONFIG=/app/config.toml
EXPOSE 8000
ENTRYPOINT ["/usr/local/bin/vela-server"]

FROM runtime AS qualification
COPY --from=qualification-build /vela-server /usr/local/bin/vela-server

FROM runtime AS production
ARG TARGETARCH
RUN case "$TARGETARCH" in amd64) ort_arch=x64 ;; arm64) ort_arch=aarch64 ;; *) exit 1 ;; esac \
    && curl --fail --location --retry 3 \
       "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-${ort_arch}-1.23.2.tgz" \
       --output /tmp/onnxruntime.tgz \
    && mkdir -p runtime/onnxruntime \
    && tar -xzf /tmp/onnxruntime.tgz --strip-components=2 -C runtime/onnxruntime \
       "onnxruntime-linux-${ort_arch}-1.23.2/lib" \
    && rm /tmp/onnxruntime.tgz
COPY --from=release /vela-server /usr/local/bin/vela-server
USER 1000:1000
