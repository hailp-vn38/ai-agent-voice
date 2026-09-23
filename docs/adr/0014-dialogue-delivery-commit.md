# ADR 0014 — Dialogue chỉ commit assistant đã được deliver

## Status
Accepted

User utterance được commit sau ASR final hợp lệ. `SpeechOutput::Drained` chỉ kết thúc pipeline speech vào outbound path. Generated Assistant Response chỉ trở thành Delivered Assistant Response và được commit khi WebSocket writer trả `WriterEvent::TurnClosed { outcome: Normal }` cho đúng `TurnId`; response bị cancel hoặc lỗi không được ghi như đã nói. Đây xác nhận server writer đã đóng turn, không xác nhận client đã phát xong audio.
