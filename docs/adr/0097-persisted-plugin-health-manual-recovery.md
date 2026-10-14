# ADR 0097 — Persisted fail-stop Plugin status with Admin-only recovery

## Status
Proposed, 2026-10-13.

## Decision
Installed executable plugins retain one SQLite state `disabled|ready|error`, revision and safe last-error metadata. Install publishes **disabled** only after package/image build validation. Admin **Test** is diagnostic and never changes status; verified **Activate** is the only `disabled|error -> ready` transition. Admin **Disable** and Uninstall prevent new starts; uninstall/update refuses while nonterminal Jobs/active workers pin the image. Admin approval is stored per package digest in installation metadata, not a new per-Agent grant table.

A fatal plugin-specific import/dependency mismatch, package/image integrity problem or worker protocol failure transitions `ready -> error` atomically and suppresses further dispatch. Bad LLM args, empty search, external HTTP 429/5xx, transient upstream outage, Job cancel and global runner downtime are **not** evidence that a particular plugin package is broken; record Step/infrastructure failure but do not poison all plugin status rows. Admin must explicitly reset `error` with verified Activate; restart/test/network recovery never auto-heals.

All worker starts check live Job-origin Device+Agent assignment and enabled state, global settings, pinned package/image, reviewed permission subset and current installation status. Coordinate using existing `tool_security.publication` read/write primitive around authorization and **actual start**; it provides synchronization only, not MCP approval. No new Background MCP identity is created. Once started, a worker may act after revocation; best-effort kill and `unknown_outcome` evidence are mandatory.

## Why / consequences
Persistent fail-stop makes broken code and dependency builds visible without recurring hidden retry storms; careful classification avoids disabling all plugins for one host outage. Global activation is a broad capability for every admitted Agent. Permission review must be explicit and sandbox policy must genuinely enforce host/network restrictions; manifest claims are never proof of safe behavior.

## Integration clarification v1.5.1
Plugin-specific fatal failures alone transition to `error`; shared runner or image-builder outages make the runtime unavailable without poisoning all plugins. Activate network-required code fails closed unless a deployed enforced egress policy passes preflight; package Build and Run are both isolated.
