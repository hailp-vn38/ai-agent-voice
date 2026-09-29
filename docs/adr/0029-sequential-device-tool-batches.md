# ADR 0029 — Device tool batch chạy tuần tự theo LLM order

## Status
Accepted — superseded in part by ADR 0060

Nhiều tool call trong một LLM round chạy tuần tự theo thứ tự response, mỗi call có timeout riêng; tool-level error không ngăn sibling còn lại khi session/generation vẫn khỏe. Tool result trả lại đúng thứ tự và tool_call_id. `max_tool_depth` đếm tool round, không đếm individual call; V1 không parallelize Device MCP để giữ observable ordering.

## Superseded in part

ADR 0060 giữ nguyên quyết định này và mở rộng nó: một Tool-round Executor chung thay cho batch chỉ của
Device MCP, nên cả External MCP ToolCall và session-local action cũng chạy trong cùng thứ tự tuần
tự. Tên cấu hình `max_tool_depth` được đổi thành `llm.tools.max_rounds_per_turn` và giữ nguyên
cách đếm (tool round, không đếm individual call); hai cap mới — `llm.tools.max_calls_per_round` và
`llm.tools.execution_budget_ms` — được thêm vào. ADR 0029 không còn mô tả cấu hình hiện hành.

