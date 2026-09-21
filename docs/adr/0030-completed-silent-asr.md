# ADR 0030 — ASR text rỗng là CompletedSilent

## Status
Accepted

ASR thành công nhưng `text.trim()` rỗng là terminal `CompletedSilent`, không phải provider failure: không STT/LLM/TTS/dialogue, release Active Turn permit, rồi manual về Ready và auto về Listening. Telemetry chỉ ghi metadata `completed_silent`, không lưu text.
