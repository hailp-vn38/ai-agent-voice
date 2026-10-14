# ADR 0102 — Shared plugin image with separate installer and runner

**Status:** Accepted (implementation in progress).

## Decision
Replace per-plugin Docker images and per-attempt Docker containers with one versioned `voice-plugin-runtime` image deployed as **two long-lived containers**: `plugin-installer` and `plugin-runner`. Installer prepares immutable Python virtual environments or trusted prebuilt Rust ELF executables. Runner starts bounded process-group attempts. Server `PluginStore` keeps the authority for admission, activation, revision checks, frozen installation choices and validated artifacts. The two private RPC endpoints run over Unix-domain sockets. Neither container needs access to Docker Engine, daemon-in-daemon or an application-managed proxy.

## Context and trade-off
Image-per-plugin deployment required an additional rootless Docker Engine, pinned builder images, offline caches and container creation per attempt. A single shared runtime reduces deployment and installation complexity, but intentionally provides **less isolation between trusted plugins**. Python `.venv` isolates dependencies, not privilege. This is an **Admin-trusted Plugin Store**, not a public marketplace.

## Identity and lifecycle
`Plugin Package` is immutable ZIP content identified by package digest. `Plugin Installation` is a prepared, immutable environment identified by `installation_digest`, not the shared OCI image. Activation is explicit Admin action. `Worker Attempt` is identified by `attempt_id` and must not be executed twice after a lost acknowledgement. Preserve the existing `jsonl-v1` plugin contract and Job state machine; process status from Runner is evidence, not an authority to update Job state itself.

The v2 manifest adopts runtime kinds `python` and `rust`. Production Rust plugins are prebuilt Linux binaries; compiling Rust source is a CI concern. A forward migration disables legacy per-image installations and requires reinstall. Drain or terminalize old active Jobs before switching runtime; completed history stays intact. Rollback requires restoring a compatible database snapshot.

## Networking and other ADRs
ADR 0101 remains authoritative: plugin workers use direct container networking without `network_hosts`, egress proxy, or approval grants. External MCP networking is unchanged. This ADR supersedes the execution-container and image-build decisions in ADR 0093 (Python) and ADR 0099 (Rust); the remaining Job scheduling/admission decisions stand.

## Consequences
Installer cannot write active Runner packages once prepared; Runner mounts installations read-only and writes per-attempt output only. No Docker socket or server credentials are mounted. Only trusted packages should be activated because shared-runtime plugins can access reachable network targets and share CPU/memory limits. Production qualification must demonstrate installation, runtime execution, cancellation, crash recovery and Voice/Job regressions on the actual Linux deployment.
