# 12: Quan sát và approve Device tool contracts

**What to build:** Admin inspect contract do Device đã admit quảng bá, approve đúng observation và dùng tool trên Agent mà không kế thừa quyền của Device khác.

**Blocked by:** 11: Review và giới hạn External MCP tools.

**Status:** resolved

- [x] Bounded completed observations lưu SQLite từ existing admitted MCP discovery; UI có timestamp và cảnh báo observed≠online/behavior guarantee, không schema Admin nhập.
- [x] API identity device_id + original name, internal Device incarnation FK, no wildcard/display name/Client ID; delete/recreate không kế thừa approvals.
- [x] Approve CAS incarnation + observed revision + fingerprint + allowlist revision trong transaction; observation đổi trả conflict.
- [x] Contract drift hoặc conflicting valid observations block entry/close affected WS, no latest-wins/no incomplete merge. Only approved tools advertised + guarded dispatch.
- [x] Public tests Device/External same-name collision, two same-Device WS observations, concurrent review, stale schema và restart metadata; UI/API docs tích hợp.

## Answer

Migration `0010_device_tool_allowlist.sql` adds `device_tool_observations` (PK `device_id, original_name`; FK to `devices.id`, the AUTOINCREMENT incarnation) and `agent_device_tool_allowlist` (PK `agent_id, device_id, original_name`). `Database` gains `device_tool_allowlist::{observe, load_admitted}` and `ToolSecurity::{register_device, invalidate_device}`; `DeviceToolGuard` carries the admitted snapshot and re-checks the live approval immediately before dispatch.

A completed, validated `tools/list` walk from the admitted Device WebSocket records evidence out of band (`session/actor/mcp.rs`), then derives the visible catalog from the admitted snapshot: only the approved fingerprint is published, and an approved name that now advertises a different contract is drift, not a new right. Admission (`app/state.rs`) resolves the snapshot and registers the cancellation token; a conflict, revocation or re-enrolment invalidates the affected sessions (close 1008).

Conflicting valid observations set `blocked=1` and keep the original evidence instead of overwriting it; a later agreeing discovery never clears the block (only a recovery batch may, a later ticket). Admin review is `GET/PUT /api/admin/agents/{key}/device-tool-allowlist`, keyed by the public `device_id` + `original_name`, with `If-Match` CAS on the approval revision and a transactional check of the observation revision and fingerprint. The Admin UI is `AgentDeviceToolAllowlist.vue`, and `docs/api/tool-allowlist.md` documents the Device surface.

Tests: `tests/device_tool_review.rs` (observation-only admits nothing, approval admits exactly the fingerprint, unreviewed denied, drift revokes/blocks, conflict is sticky, non-participating observes but keeps legacy behavior, delete/re-enrolment does not inherit, Device/External same-name collision) and the admin CAS/`observed_only` test in `tests/admin_api.rs`.

Remaining gap: the "recovery batch" that resolves a conflict is Ticket 13 (`13-discovery-conflict-recovery.md`), and the `deadline`/batch-state machinery it needs is not built here. Drift during an active session is denied and the tool blocked; the session is not proactively closed until the next invalidation reaches it.

