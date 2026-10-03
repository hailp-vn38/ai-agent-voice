# ADR 0069 — RMCP là protocol engine cho External MCP

## Status

Accepted

SQLite `mcp_servers` và `agent_mcp_bindings` là Database Desired Configuration source of truth cho External MCP homelab. Tại WebSocket admission, application đọc một snapshot DB đã validate, resolve secret đúng một lần và materialize `ExternalMcpClient`/Session Tool Catalog immutable; `rmcp` không nhận `Database`, repository hay DB row, còn `SessionActor` không query hay re-resolve configuration trong tool hot path. Chọn published `rmcp` release làm sole protocol engine cho modern single-endpoint Streamable HTTP (`application/json` và `text/event-stream`), bao gồm lifecycle `initialize`, paginated `tools/list` và `tools/call`; không hỗ trợ legacy two-endpoint HTTP+SSE và không giữ manual/fallback protocol engine song song. Quyết định này thay thế JSON-RPC/SSE framing tự viết để giảm protocol surface phải tự duy trì, nhưng không chuyển ownership security hay Voice semantics sang SDK.

Application vẫn sở hữu External MCP outbound network policy, DNS/CIDR revalidation, TLS/redirect restrictions, static-header/MCP-header/typed-auth ordering, SecretValue redaction, process-global limiter, turn budget/cancellation, typed content-free failure mapping, tool schema/name validation và immutable snapshot lifecycle. Vì vậy DB row không thể thành arbitrary HTTP client, SDK không được retry/recover/concurrently issue requests trái one-attempt Tool-round contract, và Admin mutation/secret rotation/runtime call failure không mutate session đã admit. ADR này bổ sung, không supersede ADR-0050 (desired versus loaded), ADR-0052 (immutable profile), ADR-0053 (fail-soft), ADR-0056 (outbound policy), ADR-0059 (schema subset), ADR-0060 (tool executor) hay ADR-0061 (secret lifecycle).
