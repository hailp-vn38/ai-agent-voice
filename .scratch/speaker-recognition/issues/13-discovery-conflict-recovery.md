# 13: Giải quyết conflict bằng discovery theo đợt

**What to build:** Admin khởi động đợt discovery recovery cho Device và chỉ review lại khi mọi observation cần thiết hoàn tất, nhất quán.

**Blocked by:** 12: Quan sát và approve Device tool contracts.

**Status:** resolved

- [x] Reuse MCP tools/list, scoped incarnation + batch identity/deadline/states; không protocol mới hoặc product wizard.
- [x] Chỉ complete observations current batch được xét; stale prior-batch bị loại. Conflict/timeout/incomplete tiếp tục blocked, không bỏ failed members để claim consistent.
- [x] Observations cũ superseded theo retention bounded; successful batch chỉ reviewable, vẫn conditional approve mới, không tự restore allowlist.
- [x] UI/API diễn đạt batch outcome/time và review state; WS/control vẫn theo admission hiện có.
- [x] Public multi-WS/HTTP races kiểm late results, mixed batches, disconnect/timeout, consistency, revision conflicts và bounded retention.

## Answer

Implemented on branch `speaker/13` (commit d3c1b1c), rebased onto current `integration/speaker-recognition` and merged.

- Migration `0011_device_tool_recovery.sql`: `device_tool_recovery_batches` (identity, deadline, explicit state), `device_tool_recovery_members` (one complete walk per Session, batch-scoped), `device_tool_observation_history` (superseded evidence).
- `src/database/device_tool_recovery.rs`: `start`/`record`/`current`/`fail_open`/`sweep_expired`/`latest_for_agent`. A batch resolves only when ≥2 distinct complete members agree AND every currently-blocked tool was re-observed; conflict/missing/disconnect/timeout keeps it blocked. Resolution clears the block, adopts the agreed contract, supersedes old evidence; caps batches (8) and history (32) per device.
- Actor wiring `session/actor/mcp.rs` + `session/actor/lifecycle.rs`; admin API `POST /api/admin/agents/{key}/device-tool-recovery`; allowlist `GET` returns a `recovery` map; UI `AgentDeviceToolAllowlist.vue`.
- Tests: `tests/device_tool_recovery.rs` (8), `tests/device_tool_review.rs` (8), `tests/admin_api.rs` (+1), `tests/device_mcp.rs` (4). All green.

Remaining gap (documented in code): no end-to-end multi-WS harness driving a real device `tools/list` through the actor; disconnect handling is conservative safe-fail (fails the open batch rather than detecting quiescence). Both ceilings noted for a later slice.
