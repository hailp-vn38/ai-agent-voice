# ADR 0018 — Tool-capable LLM round phải buffer trước TTS

## Status
Accepted

Khi LLM request có LLM-visible Tool, server buffer prose và tool call đến hết round. Round có tool call không được TTS; server chỉ synthesize final round không có tool call sau khi tool result đã quay lại model. Điều này hy sinh token-to-TTS streaming cho tool turn để không phát lời dẫn hoặc kết quả chưa đúng.
