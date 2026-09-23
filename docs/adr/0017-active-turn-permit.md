# ADR 0017 — Tách ASR Stream Lease khỏi Active Turn permit

## Status
Accepted

Streaming ASR bắt đầu khi thu âm nên không còn trùng với ranh giới một Conversational Turn active. Server lấy `AsrStreamLease` khi recognition stream bắt đầu (`SpeechStart` ở Auto, `listen:start` ở Manual), để giới hạn `max_asr_streams`; lease được release ngay sau ASR final, cancel hoặc lỗi.

`Active Turn Limiter` thuộc application runtime/AppState, không thuộc ProviderSet hay model provider. Tại utterance terminal boundary (`SpeechEnd` ở Auto, `listen:stop` ở Manual), session try-acquire global Active Turn permit trước `AsrSession.finish()`. Nếu không có permit, server cancel recognition stream, release `AsrStreamLease` và bỏ turn, không xếp chờ. Khi có permit, permit giữ xuyên ASR finalization, LLM, MCP và SpeechOutput đến terminal path: với playback, writer `TurnClosed`; không playback, controlled terminalization. Hai resource có overlap ngắn trong `finish()`; chúng không thay thế nhau.
