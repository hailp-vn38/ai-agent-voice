# 11: Review và giới hạn External MCP tools

**What to build:** Admin review tool của External MCP server cho Agent; chỉ tool được cho phép xuất hiện với LLM và được dispatch.

**Blocked by:** None (can start immediately).

**Status:** claimed

- [ ] Agent Tool Allowlist persistence/API/Agent UI keyed nội bộ incarnation, public server_key + original tool name; default deny, hiện có deny filters vẫn áp dụng.
- [ ] Review source/schema/description/endpoint/transport/auth scope/reference fingerprint, không secret value; conditional update không approve contract stale.
- [ ] Filter Session Tool Catalog và recheck trước common dispatch; direct names/aliases/source collisions không bypass; sensitive tool bị chặn khi chưa Independent Confirmation.
- [ ] Configuration/observed contract drift và giảm quyền chặn dispatch/close affected WS; additions chỉ WS mới, Observe không speaker authority hoặc exemption.
- [ ] Public Admin+WS tests qua External MCP double kiểm discovery→review→LLM advertisement→dispatch, revoke races và delete/recreate; tái dùng security epoch/invalidation cho các slice sau.
