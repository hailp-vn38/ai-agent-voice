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
# Media tools for YouTube/media agent tools. FFmpeg comes from Debian; yt-dlp and Deno
# (yt-dlp's JavaScript runtime for YouTube) are pinned and hash-checked. A newer yt-dlp
# placed in /app/data/bin (see `compose.sh prod tools-update`) wins over the pinned one.
ARG YTDLP_VERSION=2026.10.26
ARG YTDLP_SHA256_AMD64=767ba9dce7052aa196f8b12652f433d76de128ba168966fa84b6394d5e6c6795
ARG YTDLP_SHA256_ARM64=e87e9533e89fe5889abaa21f5114dfe7756d6aa6219b83551add04d48b18e483
ARG DENO_VERSION=2.9.8
ARG DENO_SHA256_AMD64=e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609
ARG DENO_SHA256_ARM64=d49596c20b9788cf8f5d8ddf644103d4bde56ed239753f0bdb92ba9361d51905
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg unzip \
    && rm -rf /var/lib/apt/lists/* \
    && case "$TARGETARCH" in \
         amd64) yt_asset=yt-dlp_linux; yt_sha=$YTDLP_SHA256_AMD64; deno_arch=x86_64; deno_sha=$DENO_SHA256_AMD64 ;; \
         arm64) yt_asset=yt-dlp_linux_aarch64; yt_sha=$YTDLP_SHA256_ARM64; deno_arch=aarch64; deno_sha=$DENO_SHA256_ARM64 ;; \
         *) exit 1 ;; \
       esac \
    && mkdir -p /opt/tools \
    && curl --fail --location --retry 3 --output /opt/tools/yt-dlp \
       "https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/${yt_asset}" \
    && echo "${yt_sha}  /opt/tools/yt-dlp" | sha256sum -c - \
    && curl --fail --location --retry 3 --output /tmp/deno.zip \
       "https://github.com/denoland/deno/releases/download/v${DENO_VERSION}/deno-${deno_arch}-unknown-linux-gnu.zip" \
    && echo "${deno_sha}  /tmp/deno.zip" | sha256sum -c - \
    && unzip -q /tmp/deno.zip -d /opt/tools && rm /tmp/deno.zip \
    && chmod 755 /opt/tools/yt-dlp /opt/tools/deno
ENV PATH=/app/data/bin:/opt/tools:$PATH \
    XDG_CACHE_HOME=/app/data/cache \
    DENO_DIR=/app/data/cache/deno
RUN yt-dlp --version && deno --version && ffmpeg -version | head -1 && ffprobe -version | head -1
COPY --from=release /vela-server /usr/local/bin/vela-server
USER 1000:1000
