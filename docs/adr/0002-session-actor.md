# ADR 0002 — SessionActor sở hữu mutable session state

## Status
Accepted

## Decision
Mỗi WebSocket connection tạo một `SessionActor`. Các task khác giao tiếp qua channel/event.

## Rationale
Tránh shared mutable state giữa WS, ASR, LLM, TTS, MCP và cancellation logic.

## Consequences
Mọi feature mới liên quan state phải được biểu diễn thành event/command thay vì giữ mutable reference tới session.
