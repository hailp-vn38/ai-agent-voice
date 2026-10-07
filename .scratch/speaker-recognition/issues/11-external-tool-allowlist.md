# 11: Review và giới hạn External MCP tools

**What to build:** Admin review tool của External MCP server cho Agent; chỉ tool được cho phép xuất hiện với LLM và được dispatch.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Agent Tool Allowlist persistence/API/Agent UI keyed nội bộ incarnation, public server_key + original tool name; default deny, hiện có deny filters vẫn áp dụng.
- [x] Review source/schema/description/endpoint/transport/auth scope/reference fingerprint, không secret value; conditional update không approve contract stale.
- [x] Filter Session Tool Catalog và recheck trước common dispatch; direct names/aliases/source collisions không bypass; sensitive tool bị chặn khi chưa Independent Confirmation.
- [x] Configuration/observed contract drift và giảm quyền chặn dispatch/close affected WS; additions chỉ WS mới, Observe không speaker authority hoặc exemption.
- [x] Public Admin+WS tests qua External MCP double kiểm discovery→review→LLM advertisement→dispatch, revoke races và delete/recreate; tái dùng security epoch/invalidation cho các slice sau.

## Answer

Implemented on branch `speaker/11` (commits ad18ad0, 11bb408), rebased onto current `integration/speaker-recognition` and merged.

- Migration `0008_agent_tool_allowlist.sql`; `src/database/tool_allowlist.rs` (incarnation-keyed allowlist) and `src/database/tool_security.rs` (contract fingerprint / security epoch).
- Admin API `src/app/admin/tool_allowlist.rs` + review hooks in `mcp_servers.rs`; default deny; conditional update rejects stale contract approval; secret values never surfaced.
- Session tool catalog filtering + recheck before common dispatch (`session/actor/tools/external.rs`, `session/actor/ingress.rs`); sensitive tools blocked without Independent Confirmation; contract drift revokes/deny and closes affected WS.
- Agent UI `apps/admin-web/src/components/agents/AgentToolAllowlist.vue` wired into `AgentDetailPage.vue`.
- Tests: `tests/external_mcp_admission.rs` (16), `tests/tool_round_executor.rs` (12, discovery→review→advertisement→dispatch, revoke races), `tests/admin_api.rs` (+1). All green.
- Docs: `docs/api/tool-allowlist.md` + Postman collection entries.

Note: ticket 05's speaker migration is being renumbered to `0009` at merge time so both keep distinct sequence numbers.
