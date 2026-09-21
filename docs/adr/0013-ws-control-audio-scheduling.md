# ADR 0013 — WS writer ưu tiên control hơn audio

## Status
Accepted

WS writer sở hữu hai bounded queue `control_tx` và `audio_tx`, luôn xử lý control hợp lệ trước audio. Audio mang Generation ID và bị kiểm tra trước enqueue lẫn trước WebSocket send; `tts:start` của generation phải đến trước Opus đầu tiên, còn sau `tts:stop` của generation đó không Opus nào được tới WebSocket. Điều này giữ một writer duy nhất nhưng cho phép stop vượt audio stale khi abort.
