# ADR 0101 — Direct network access for executable Agent Job plugins

## Status
Accepted for the Agent Job plugin runtime, 2026-10-13.

Supersedes the plugin network approval and egress-proxy decisions in ADR 0093 and the Agent Job integration notes. External MCP outbound policy is independent and unchanged.

## Decision
- `plugin.toml` has no `[permissions]` block and no `network_hosts` setting. An unknown `permissions` field is invalid in a new package.
- Installation is followed by an explicit, revision-guarded **Activate** action. There is no extra network/storage permission approval API or UI.
- Each Python/Rust plugin step starts in a short-lived rootless Docker worker **on the Engine's default network**. The server does not create an egress proxy, allocate per-step Docker networks, filter outbound hosts, or inject HTTP(S)_PROXY variables. The worker can reach services allowed by Docker and the host network.
- Network access does not imply access to host mounts or the Docker socket. Keep non-root execution, read-only root filesystem, dropped capabilities, no-new-privileges, resource limits, a bounded artifact mount, timeouts, and ownership-based worker reaping.
- Build stages remain offline (`docker build --network=none`, locked dependencies), independently of the runtime network choice, to maintain reproducible builds.
- Frozen Job choices pin plugin ID, version, package digest, image digest, and operation. No permission digest or approval snapshot is stored or checked.
- Remove the proxy image configuration, deployment build script, API permission endpoint, and approved-permission database column via a forward migration.

## Consequences
**Outbound connections are unrestricted by the application**, including destinations on reachable private networks. A malicious or compromised plugin can attempt requests to internal services or metadata endpoints. The server does not promise SSRF prevention or network isolation. Install only trusted plugin code; apply external host or infrastructure policy only if the deployment independently requires it.

Old installed plugin packages and pending Job snapshots with permission claims are deliberately **not supported** by this change. Repackage/reinstall plugins and clear obsolete queued Jobs if necessary; no compatibility layer is provided. External MCP policy and the offline image-build boundary remain separate.
