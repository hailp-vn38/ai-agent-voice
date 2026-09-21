# ADR 0014 — Dialogue chỉ commit assistant đã được deliver

## Status
Accepted

User utterance được commit sau ASR final hợp lệ. Generated Assistant Response chỉ trở thành Delivered Assistant Response và được commit vào dialogue sau khi SpeechOutput `Drained`; response bị cancel hoặc lỗi không được ghi như đã nói, dù có thể giữ riêng cho telemetry. Điều này ưu tiên context trung thực với điều người dùng thực sự đã nghe.
