# ADR 0009 — Bounded concurrency và fail-fast overload

## Status
Accepted

V1 giới hạn mặc định bốn kết nối, hai active turn và hai concurrency slot cho từng ASR, LLM, TTS; mọi queue đều bounded. Quá giới hạn kết nối đóng/từ chối với 1013; turn không có slot bị từ chối; audio outbound đầy hoặc provider lỗi/timeout hủy toàn generation thay vì drop audio tùy ý. Control có ưu tiên hơn audio; control channel không còn hoạt động thì đóng WebSocket.
