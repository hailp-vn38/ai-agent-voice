# ADR 0016 — Tách Ready khỏi Listening

## Status
Accepted

Sau `tts:stop`, manual mode chuyển Voice Session sang Ready còn auto mode chuyển sang Listening. Ready giữ WebSocket, dialogue và MCP nhưng không accept hay accumulate microphone audio; Listening mới kích hoạt audio collector/VAD. Tên Ready tránh nhầm trạng thái kết nối với idle của firmware.
