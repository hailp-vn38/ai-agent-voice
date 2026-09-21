# ADR 0026 — Tool-level failure tiếp tục LLM bằng result đã sanitize

## Status
Accepted

Khi Voice Session còn khỏe, MCP timeout, JSON-RPC error, `isError`, argument reject hoặc discovery race trở thành tool result `ok:false` đã sanitize để LLM tạo final no-tool response. Disconnect, session replacement, root cancellation, shutdown, generation cancellation hoặc MCP routing mất session là terminal. Khi vượt max tool depth, inject `tool_depth_exceeded` rồi chỉ cho một final no-tools LLM round.
