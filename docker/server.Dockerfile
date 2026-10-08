# syntax=docker/dockerfile:1
FROM rust:1.98.1-bookworm AS source
RUN apt-get update && apt-get install -y --no-install-recommends cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo .cargo
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
RUN --mount=type=cache,id=voice-agent-cargo,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=voice-agent-target,target=/mnt/storage/ai-agent-voice/target,sharing=locked \
    cargo build --locked --release -p voice-agent-server --bin voice-agent-server \
    && cp /mnt/storage/ai-agent-voice/target/release/voice-agent-server /voice-agent-server

FROM source AS qualification-build
RUN --mount=type=cache,id=voice-agent-cargo,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=voice-agent-target,target=/mnt/storage/ai-agent-voice/target,sharing=locked \
    cargo build --locked -p voice-agent-server --bin voice-agent-server --features qualification-providers \
    && cp /mnt/storage/ai-agent-voice/target/debug/voice-agent-server /voice-agent-server

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl libstdc++6 libgomp1 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY assets/enrollment assets/enrollment
ENV VOICE_AGENT_CONFIG=/app/config.toml
EXPOSE 8000
ENTRYPOINT ["/usr/local/bin/voice-agent-server"]

FROM runtime AS qualification
COPY --from=qualification-build /voice-agent-server /usr/local/bin/voice-agent-server

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
COPY --from=release /voice-agent-server /usr/local/bin/voice-agent-server
USER 1000:1000
