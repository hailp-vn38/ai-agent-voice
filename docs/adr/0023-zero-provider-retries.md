# ADR 0023 — Không automatic retry logical provider operation trong V1

## Status
Accepted

Mỗi logical ASR, LLM, TTS hoặc MCP operation V1 chỉ thực hiện một lần; timeout/lỗi đi theo terminal path, không retry application/provider layer. MCP `tools/call` đặc biệt không retry để tránh side effect lặp. Điều này giữ latency, cost, cancellation và delivery semantics deterministic.
