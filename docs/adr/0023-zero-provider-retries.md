# ADR 0023 — Không automatic retry logical provider operation trong V1

## Status
Accepted

Mỗi logical ASR, LLM, TTS hoặc MCP operation V1 chỉ thực hiện một lần; timeout/lỗi đi theo terminal path, không retry application/provider layer. MCP `tools/call` đặc biệt không retry để tránh side effect lặp. Điều này giữ latency, cost, cancellation và delivery semantics deterministic.

## Reminder delivery attempts (proposed)

Retry bền vững ở cấp Reminder được đề xuất trong [ADR-0090](0090-reminder-playback-ack-and-bounded-attempts.md). Mỗi attempt là một logical TTS operation mới; không thay đổi lệnh cấm retry bên trong provider operation hoặc MCP call. Đây là thiết kế chưa triển khai.
