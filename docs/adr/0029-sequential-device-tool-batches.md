# ADR 0029 — Device tool batch chạy tuần tự theo LLM order

## Status
Accepted

Nhiều tool call trong một LLM round chạy tuần tự theo thứ tự response, mỗi call có timeout riêng; tool-level error không ngăn sibling còn lại khi session/generation vẫn khỏe. Tool result trả lại đúng thứ tự và tool_call_id. `max_tool_depth` đếm tool round, không đếm individual call; V1 không parallelize Device MCP để giữ observable ordering.
