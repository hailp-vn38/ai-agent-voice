# External MCP and Device tool review

## External MCP tools

Speaker participating Agents (`observe` / `required`) default-deny External MCP tools. Off Agents retain their existing capability behavior. Complete validated admission discovery records the bounded source contract; discovery never grants rights.

`GET /api/admin/agents/{key}/tool-allowlist` returns observed original tool names, `server_key`, description/schema, endpoint/transport/auth reference (never secret values), observation time/revision, fingerprint, approval revision, `allowed`, and `sensitive`. Observation does not assert online presence. No observation means no reviewable tool.

`PUT` on the same path takes `If-Match: "<approval revision>"` (an absent approval starts at revision 1) and JSON `{server_key, original_name, observed_revision, fingerprint, allowed, sensitive}`. Observation and approval revisions are checked in one transaction; stale contract/approval returns 409. Sensitive approvals remain blocked because Independent Confirmation is unavailable in V1.

An approval addition affects new connections only. Revocation, configuration change, disappearance, and observed description/schema drift invalidate affected participating sessions with WebSocket close 1008 and block new dispatch. Rediscovering an earlier contract never restores an approval. Calls already dispatched cannot be undone. Relevant endpoint, header, auth or enabled-state configuration changes require re-review; display-name and timeout updates preserve approvals. Remote behavior changes retaining the same contract and changes before rediscovery are outside this guarantee.

The shared `Database.tool_security` publication lock coordinates approval/configuration transactions with call initiation, and its scoped cancellation registry supports subsequent Speaker security invalidations. The lock is released after outbound operation initiation; it never waits for the remote response. Resource IDs use existing AUTOINCREMENT incarnations and FK cascades, so delete/recreate cannot inherit rights.

## Device tools

Speaker participating Agents (`observe` / `required`) default-deny Device tools advertised over an admitted Device WebSocket. Off Agents retain the deployment Device allowlist. Complete, validated `tools/list` discovery records the bounded observation; discovery never grants rights.

`GET /api/admin/agents/{key}/device-tool-allowlist` returns `device_id` (the public Device Identity, never the internal incarnation id), `original_name`, description/schema, observation time/revision, fingerprint, approval revision, `allowed`, `sensitive`, and `presence: "observed_only"`. Observation does not assert online presence, and there is no Admin-entered schema: only an admitted Device's own discovery is evidence. No observation means no reviewable tool.

`PUT` on the same path takes `If-Match: "<approval revision>"` (an absent approval starts at revision 1) and JSON `{device_id, original_name, observed_revision, fingerprint, allowed, sensitive}`. The Device must belong to the Agent, the observation must be unblocked at the Device's current revision and match the fingerprint and observation revision, and the approval revision is checked in one transaction; stale contract/approval returns 409 `contract_conflict` / `revision_conflict`. Sensitive approvals remain blocked because Independent Confirmation is unavailable in V1.

Approvals are keyed by the Device's internal AUTOINCREMENT incarnation, so deleting and re-enrolling a Device (a fresh id) never inherits an approval, and a Device tool never shares rights with an External tool that happens to have the same `original_name`.

A complete discovery whose contract differs from the stored one is a conflicting valid observation: the tool is marked blocked rather than overwritten, no connection wins by arriving last, and affected sessions are invalidated with WebSocket close 1008. A later agreeing discovery never clears the conflict; only a recovery batch may (a later ticket). Only the approved fingerprint is published to the LLM, and dispatch re-checks the live approval.
