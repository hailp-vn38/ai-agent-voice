# External MCP tool review

## External MCP tools

Every Agent, including Speaker Policy `off`, default-denies External MCP tools. Complete validated admission discovery records the bounded source contract; discovery never grants rights.

`GET /api/admin/agents/{key}/tool-allowlist` returns observed original tool names, `server_key`, description/schema, endpoint/transport/auth reference (never secret values), observation time/revision, fingerprint, approval revision, `allowed`, and `sensitive`. Observation does not assert online presence. No observation means no reviewable tool.

`PUT` on the same path takes `If-Match: "<approval revision>"` (an absent approval starts at revision 1) and JSON `{server_key, original_name, observed_revision, fingerprint, allowed, sensitive}`. Observation and approval revisions are checked in one transaction; stale contract/approval returns 409. Sensitive approvals remain blocked because Independent Confirmation is unavailable in V1.

An approval addition affects new connections only. Revocation, configuration change, disappearance, and observed description/schema drift invalidate affected sessions with WebSocket close 1008 and block new dispatch. Rediscovering an earlier contract never restores an approval. Calls already dispatched cannot be undone. Relevant endpoint, header, auth or enabled-state configuration changes require re-review; display-name and timeout updates preserve approvals. Remote behavior changes retaining the same contract and changes before rediscovery are outside this guarantee.

The shared `Database.tool_security` publication lock coordinates approval/configuration transactions with call initiation, and its scoped cancellation registry supports subsequent Speaker security invalidations. The lock is released after outbound operation initiation; it never waits for the remote response. Resource IDs use existing AUTOINCREMENT incarnations and FK cascades, so delete/recreate cannot inherit rights.

## Device tools

An admitted MCP-capable Device publishes every structurally valid, unambiguous tool from a complete `tools/list` walk directly to its own Voice Session. This is independent of Speaker Policy. Discovery failure, partial discovery, duplicate original names, or sanitized-name collisions leaves no callable catalog. Calls still require the owning session, an active turn and normal execution limits.

Device tool review and recovery endpoints are retired and return `404`:

- `GET` / `PUT /api/admin/agents/{key}/device-tool-allowlist`
- `POST /api/admin/agents/{key}/device-tool-recovery`
