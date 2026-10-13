# ADR 0099 — Rust executable plugins share the existing isolated worker

## Status

Superseded in its build and execution model by [ADR 0102](0102-shared-plugin-runtime.md). The earlier claim about reading persisted Python manifests without changes is superseded by [ADR 0101](0101-direct-plugin-worker-network.md): legacy manifests containing plugin network permissions are not supported. Rust and Python share the direct runtime networking boundary in ADR 0101; Rust source builds move to CI; production installs only prebuilt binaries.

## Context

The Agent Job executable Plugin Store accepts Python packages through a validated ZIP manifest, immutable OCI images and the existing rootless container worker. Rust binaries are another implementation of the same Plugin Operation: they must not become a separate Job, MCP identity, tool transport or Agent grant. Compiling Rust packages may run build scripts and dependencies, and loading arbitrary dynamic libraries in the Voice process would weaken failure isolation.

## Decision

- Introduce a strictly validated `rust_container` manifest runtime alongside `python_container` under `manifest_version = 1`; existing persisted Python manifest JSON remains readable.
- Rust packages contain a locked, non-workspace Cargo binary project. Cargo compiles with `--release --locked --offline` inside the restricted **build** environment; absent pre-cached dependencies cause install failure rather than network fallback.
- Deployment pins an offline Rust builder and a runtime OCI base by repository digest. The server generates a two-stage Dockerfile and verifies package/OCI labels and architecture at activation and dispatch.
- Both runtimes use `PluginStore::invoke`, the current rootless `runner::Prepared`, `jsonl-v1`, Step authority, cancellation, attempt history and Artifact Store. No new table, Admin route, Web page or Worker protocol is introduced.
- Build-time package code is not trusted. No host credentials, Docker socket or host interpreter access in either build or run stage. Runtime outbound networking is governed by ADR 0101; builds remain offline.

## Alternatives and consequences

In-process Rust dynamic libraries were rejected because plugin failure and privileges would leak into the Voice process. A second Rust-specific container runner was rejected because it would duplicate authorization, lifecycle, cancellation and artifact invariants. A general multi-language plugin framework was deferred because two build paths behind the existing interface suffice.

Rust plugins require a compatible pinned Linux builder/runtime pair and an operator-managed offline Cargo cache. Build failure is explicit; a successful source build alone does not certify end-to-end rootless deployment, Voice or deployment network qualification. Python installation and running Jobs keep their original package/image pins.

## References

- ADR 0093 — Executable Python packages, not MCP-bound plugins.
- ADR 0095 — Single-process durable Job recovery.
- `crates/voice-agent-server/src/services/background/plugins/{archive,builder,dispatch,runner,protocol}.rs`.
- `docs/agent-job-platform-v1.5.1-integration/` for full functional and deployment gates.
