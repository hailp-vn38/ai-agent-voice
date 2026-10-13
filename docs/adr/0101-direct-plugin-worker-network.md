# ADR 0101 — Direct networking for executable plugin workers

## Status
Accepted — 2026-10-13. Supersedes the **plugin networking and permission-approval** parts of [ADR 0093](0093-executable-python-plugin-runtime.md) and older Agent Job integration notes. Does not supersede External MCP networking. The worker deployment and installation model is updated by [ADR 0102](0102-shared-plugin-runtime.md).

## Context
Agent Job plugins previously declared `network_hosts` in `plugin.toml`, required a separate Admin permission approval, and received outbound access through a per-attempt egress proxy. This tied plugin installation, Job eligibility, network policy and worker lifecycle to one host allowlist. We chose a simpler, deployment-trusted executable Plugin Store: the server controls **which installed plugin may run**, not which destination that plugin may contact.

## Decision
- **Plugin Package** declares runtime, agent instructions, operations and schemas, **not network permissions**. Neither `[permissions]` nor `network_hosts` is part of the accepted manifest. There is no host grant, approved-host snapshot or `permission_digest`.
- **Plugin Installation** initially has status `disabled`. **Plugin Activation** is an explicit Admin action guarded by the installation revision; it validates the package/image and makes operations eligible for Jobs. Activation is **not** a destination-specific network approval or a security audit of plugin behavior.
- Every Python or Rust **Worker Attempt** is a subprocess in the long-lived shared Runner container attached to the Compose default runtime network (ADR 0102). The server does not create per-attempt networks, deploy an egress proxy, inject `HTTP_PROXY`/`HTTPS_PROXY`, or apply host/IP/domain allowlists. Any destination accessible from that Docker network is reachable in principle, including private services.
- **Plugin Network Reachability** is an outcome of Docker/host connectivity, not a field of Plugin Package, an Agent grant or an Admin setting. It does not imply a successful request: DNS, routes, the remote endpoint or infrastructure can still prevent a connection.
- Keep process and container protections: non-root Runner, read-only installation mount, cap-drop/no-new-privileges, bounded resources/time/output and controlled artifact mount. These are **not** strong per-plugin isolation because all attempts share one container. No server credentials, DB file or Docker socket are mounted.
- **Build-time network** is separate: Installer resolves Python dependencies from a frozen `uv.lock`, with direct deployment networking. Production Rust binaries are compiled in trusted CI, not on the plugin runtime. The Admin Test endpoint validates the prepared installation rather than fetching each operation URL.
- **Frozen Plugin Choices** pin plugin ID, version, package digest, installation digest and operation. Admission, start and completion recheck the pinned installed plugin's `ready` status, without a network grant or permission digest.
- Remove the plugin-permissions endpoint/UI, egress configuration and proxy dependency, and the `approved_permissions_json` column through a forward migration. Old installed packages and old queued Job snapshots are explicitly unsupported; no compatibility adapter is required.

## Boundary scenarios
1. **Web Extractor** may request a public host, localhost, a private address or a non-standard HTTP(S) port if Docker networking permits it. Its HTTP(S) scheme/credential checks and no-redirect behavior are operation logic, not a server network policy.
2. **Document Analysis** does not need outbound requests but runs under the same worker networking rule. There is no per-plugin `network=none` setting.
3. **Offline checker succeeds, live fetch fails**: the package/runtime qualification may still be correct; live network behavior is tested separately.
4. **An installation is disabled or replaced during a Job**: an unchanged frozen choice does not bypass the recheck of package/image identity and `ready` status.
5. **External MCP** retains its own URL/hostname policy. It is not governed by this ADR and must not be modified as a side effect of deleting Plugin `network_hosts`.

## Trade-off and consequences
Rejected alternatives were retaining manifest host grants, keeping a server-managed proxy without manifest declarations, and forcing `network=none` for all plugin operations. They add policy/operations complexity or prevent network-aware plugins. The chosen model deliberately trades application-level egress restrictions for simpler direct worker connectivity.

**Security consequence:** an installed plugin can attempt connections to reachable internal services and metadata endpoints; the server does not promise SSRF protection or outbound network isolation. Only Admin-trusted code should be installed. No new runtime network-policy configuration is introduced. The resource/isolation constraints above remain distinct and mandatory.
