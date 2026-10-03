# ADR 0060 — Shared Tool-round Executor chạy tuần tự và bounded

## Status

Accepted

Một executor chung validate cap toàn bộ round trước execution, rồi chạy cả Device MCP và External MCP ToolCall tuần tự theo model order, tạo đúng một terminal ToolResult cho mỗi completed call; tool-level failure không dừng round, còn cap/budget/cancellation terminalize turn trước side effect mới. Config layer chỉ nhận `max_calls_per_round` `1..=32` (default 8), `max_rounds_per_turn` `1..=8` (default 4) và `execution_budget_ms` `1..=120_000` (default 30.000 ms); config invalid fail startup trước listener. Cancellation gate TurnId/GenerationId discard late response, không retry. External result thành công chỉ nhận Text hoặc Structured JSON canonical, tối đa 16 KiB, không partial accept/truncate/log/persist; tool result lỗi/success đều giữ LLM continuation không dangling.
