# ADR 0039 — Phase 3 dùng VAD và ASR streaming local

## Status
Accepted

Phase 3 dùng `silero_onnx` local Rust cho VAD probability-level và `zipformer_sherpa` local Rust cho ASR streaming; không có Python sidecar hoặc HTTP ASR trong baseline. `VadSegmenter` vẫn sở hữu endpoint semantics, còn `AsrStreamLease` giới hạn recognition stream độc lập với `Active Turn` permit. Điều này giữ latency và failure boundary local, đồng thời giữ `AsrProvider` mở cho adapter HTTP/offline trong tương lai mà không thay SessionActor hay WebSocket V1.
