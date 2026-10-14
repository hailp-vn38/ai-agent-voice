# ADR 0093 — Executable Python packages, not MCP-bound plugins

## Status
Partially superseded for plugin networking, manifest permissions and Admin permission approval by [ADR 0101](0101-direct-plugin-worker-network.md), accepted 2026-10-13. The decision text below is retained as historical context, **not the current network or approval contract**. The independent executable Python plugin and JSONL contract remain applicable. Per-plugin image building and per-attempt container execution are superseded by [ADR 0102](0102-shared-plugin-runtime.md); the decisions below are historical. External MCP policy is unaffected.

## Decision
V1 Plugin Store installs Admin-trusted, versioned **Python 3.12 executable ZIP packages**, either manually uploaded or curated deployment packages. Required files: `plugin.toml`, `pyproject.toml`, `uv.lock`, source, input/output schemas. Installer validates archive and lock, builds an isolated environment/image with pinned Python and uv, verifies image digest and commits one disabled installation per plugin ID. Admin reviews permissions and activates. The installed package and image are immutable by digest; upgrade/uninstall blocks while nonterminal Jobs pin the old one.

The Rust Plugin Store runs a **single short-lived container worker per Step attempt**, communicating over bounded JSON Lines stdin/stdout and importing output files through server-controlled Job Artifact Store. Python code performs its own logic without using External MCP Manager or server MCP tools; there is no `mcp_binding`, server-owned Background MCP identity or MCP grant. Other existing MCP paths remain available to normal Voice clients unchanged.

Runtime is rootless/user-namespaced, non-root, read-only root filesystem, cap-drop/no-new-privileges/seccomp, bounded CPU/memory/PIDs/timeout/output and restricted workspace. Default network none; networked crawling requires a deployment-enforced egress policy, not just TOML hostnames. Neither installer nor worker may access host DB/credentials/container socket. Approved code is **admin-trusted**; container does not constitute a guarantee of safety for adversarial arbitrary code. Remote/public marketplace, plugin secrets, arbitrary Dockerfiles, additional languages and warm pools are deferred.

## Why / consequences
Real Python code/dependencies allow independent crawler/search/transform logic not expressible as mappings to existing MCP tools. In-process Python/shared host virtualenv was rejected due to dependency collisions, crashes and privileges. Separate worker processes with venv alone were rejected as insufficient isolation. Container setup has operational cost, and dependency build scripts are executable: isolate **build time** as well as run time. No `uv sync` or package downloads per Job. Global Admin grants cannot exceed manifest or enforced deployment capabilities. `safe_read` is an operator review classification, not cryptographic proof against code side effects.

## Integration clarification v1.5.1
Network-free workers use `network=none`; networked workers require an isolated network with enforced egress proxy/firewall and no bypass. A proxy cannot be accessed from a network-free namespace. Install is a separate bounded build slot with finite timeout, never holding long SQLite transactions or Voice resource permits.
