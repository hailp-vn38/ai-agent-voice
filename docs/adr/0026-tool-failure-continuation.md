# ADR 0026 — Tool-level failure tiếp tục LLM bằng result đã sanitize

## Status
Accepted — superseded in part by ADR 0060

Khi Voice Session còn khỏe, MCP timeout, JSON-RPC error, `isError`, argument reject hoặc discovery race trở thành tool result `ok:false` đã sanitize để LLM tạo final no-tool response. Disconnect, session replacement, root cancellation, shutdown, generation cancellation hoặc MCP routing mất session là terminal. Khi vượt max tool depth, inject `tool_depth_exceeded` rồi chỉ cho một final no-tools LLM round.

## Superseded in part

Câu cuối không còn đúng và đã từng không đúng với code: vượt cap terminalize turn chứ không cho thêm
round nào. ADR 0060 ghi rõ cap, cap-per-round và hết Tool Execution Budget đều là turn-level
terminal failure, không synthetic ToolResult và không continuation, và đổi tên `max_tool_depth`
thành `llm.tools.max_rounds_per_turn` với class `tool_round_limit_exceeded`. Phần còn lại của ADR
này — một tool-level failure khi session khỏe vẫn trở thành ToolResult đã sanitize để LLM tiếp tục —
vẫn đúng.
