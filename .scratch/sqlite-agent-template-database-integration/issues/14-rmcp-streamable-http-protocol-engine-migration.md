# 14: RMCP Streamable HTTP protocol-engine migration

**What to build:** Dùng SQLite `mcp_servers` và `agent_mcp_bindings` làm source of truth cho homelab External MCP desired configuration, rồi thay protocol engine tự viết bằng `rmcp` official Rust SDK cho modern Streamable HTTP. Giữ contract snapshot, Tool-round và deployment security boundary của Voice Agent.

**Blocked by:** 09: External MCP admission snapshot; 10: Shared Tool-round Executor.

**Status:** resolved

**Architecture:** ADR-0069.

- [ ] Pin một release `rmcp` published, reviewable trong Cargo.lock với feature tối thiểu cho client Streamable HTTP và client-side SSE; không dùng git `main`, không thêm runtime feature flag hoặc fallback production engine.
- [ ] Admission đọc đúng một DB snapshot của enabled `mcp_servers`/`agent_mcp_bindings`, validate desired configuration rồi materialize `ResolvedExternalMcp`/`ExternalMcpClient` immutable. `rmcp` không nhận `Database`, repository hay DB row; SessionActor chỉ giữ client snapshot, không query/re-resolve config trong tool hot path.
- [ ] `ExternalMcpClient` dùng `rmcp` làm sole JSON-RPC/lifecycle/pagination/SSE protocol engine cho `initialize → tools/list → tools/call`; xóa parser/framing/session handling duplicate của engine cũ. Custom code còn lại chỉ là adapter từ domain contract sang SDK.
- [ ] Chỉ support modern single-endpoint Streamable HTTP với `application/json` và `text/event-stream`. Không hứa support legacy two-endpoint HTTP+SSE transport `2024-11-05`; configuration `transport = streamable_http` đã có không đổi. JSON, SSE, paginated catalog và session-capable modern server đều có deterministic fixture.
- [ ] Outbound policy vẫn nằm trước và ngoài SDK: implement/adapt `rmcp` HTTP backend để hostname/CIDR DNS revalidation, HTTPS validation, explicit HTTP-LAN exception, no redirect, URL restrictions, static-header → MCP-header → typed-auth ordering và SecretValue redaction vẫn apply trước mỗi outbound attempt. Failing policy phải chứng minh zero request rời process; DB row không được trở thành arbitrary HTTP client.
- [ ] Giữ `ExternalMcpManager`, `ResolvedExternalMcp`, `SessionExternalMcp`, `ToolOrigin`, validator/sanitizer và immutable Session Tool Catalog. Secret chỉ resolve lúc admission và thuộc client snapshot; Admin mutation, secret rotation, runtime call failure và DB degradation không mutate snapshot hay refresh/re-resolve credential.
- [ ] Configure/adapter SDK để không tạo retry, recovery retry hay outbound concurrency trái one-attempt + Tool-round contract. Per-server global limiter, remaining turn budget, cancellation, typed content-free failure mapping và strict sequential executor vẫn owned bởi code ứng dụng; deterministic test đếm đúng một outbound attempt cho timeout, network error, remote 401/403 và cancellation.
- [ ] A remote HTTP `401`/`403`/non-2xx hoặc malformed `tools/call` result là terminal outcome của **đúng invocation đó**, không làm RMCP client/session snapshot unusable. Test theo chuỗi `failure → malformed result → valid result` phải chứng minh từng call đều tới server đúng một lần, map lần lượt sang typed domain failure chính xác rồi success; không reinitialize session, re-discover catalog, refresh secret hay retry request giữa các call.
- [ ] Remove dead custom protocol paths in the same change; no dual production engines, mixed `rmcp`/manual lifecycle or test-only bypass. Preserve only domain-level tests/adapters that assert contracts independent of SDK internals.
- [ ] Run deterministic compatibility/security regression suite: JSON/SSE lifecycle, pagination, schema/name/result caps, redirect/DNS/header/auth rejection, no credential telemetry, fresh discovery every admission, immutable catalog, limiter/budget/cancellation and mixed Device/External ordering. Run `cargo fmt --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace`.

## Notes

SQLite is the homelab source of truth for per-server desired configuration: URL, typed auth and
secret reference, static headers, timeouts, enabled state and Agent binding. TOML remains only for
deployment-owned concerns that cannot safely be supplied by an Admin DB row: database/API bootstrap,
feature enablement and process-wide safety ceilings/policy. `rmcp` is the protocol engine, not the
configuration store or outbound policy owner. The SDK exposes a client-agnostic Streamable HTTP
transport/backend seam; the adapter must therefore be the only route from a materialized DB snapshot
to an HTTP request. This ticket intentionally migrates no Admin schema, Agent binding semantics or
Tool-round policy.

The official SDK currently targets modern MCP Streamable HTTP and documents no legacy two-endpoint
HTTP+SSE transport for protocol `2024-11-05`. Since this project already declares only
`streamable_http`, the migration makes that existing boundary explicit rather than silently carrying
a partial legacy implementation.

## Comments

- 2026-09-30: Commit `d12c28f` pins published `rmcp 3.5.0` and replaces the handwritten External
  MCP JSON-RPC/lifecycle parser with the RMCP Streamable HTTP client plus a policy-owning backend.
  The adapter keeps DNS/CIDR revalidation immediately before outbound I/O, redirect prohibition,
  static/typed-auth snapshot ownership and bounded response handling outside RMCP.
- Regression `external_mcp_tool_catalog_survives_every_call_failure` đã pass và chứng minh mỗi
  `401`/`503`, malformed result, oversized result và timeout chỉ tới mock đúng một lần; call hợp lệ
  sau đó vẫn tới được mock. `ServiceError::UnexpectedResponse` tại seam `tools/call` được map thành
  `external_tool_invalid_response`, còn transport/network vẫn là `external_tool_unavailable`; không
  có reinitialize, rediscovery, secret refresh hay request retry.
- 2026-09-30: Đã hoàn tất hai gate còn thiếu. Fixture
  `external_mcp_session_capable_sse_keeps_its_session_across_lifecycle` trả SSE thực và mint
  `mcp-session-id` ở `initialize`; fixture từ chối nếu notification `initialized`, `tools/list`
  hoặc `tools/call` không gửi lại identity đó. Regression workspace là test `GatedServer` dựng
  client chưa RMCP-initialize rồi chờ call vô hạn; fixture nay initialize trước dispatch và chính
  wait được bound 5 giây.
- Validation pass: `cargo test -p voice-agent-server --test external_mcp_admission` (15/15),
  `cargo test --workspace --quiet -- --test-threads=1`, `cargo fmt --check`,
  `cargo clippy --workspace --all-targets` (exit 0, còn warning) và `git diff --check`.
  Không còn gate acceptance mở; trạng thái `resolved` có bằng chứng workspace hiện tại.
