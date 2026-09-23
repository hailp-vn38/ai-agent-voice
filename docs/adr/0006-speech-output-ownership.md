# ADR 0006 — SpeechOutput sở hữu audio pipeline, actor sở hữu outbound ordering

## Status

Accepted

## Context

Streaming TTS cần synthesize, resample, Opus encode, queue và pacing. Nếu pacer hoặc MCP gửi trực tiếp vào outbound queue, ordering, cancellation và `tts:stop` bị phân tán giữa nhiều caller. `TtsFinished` theo generation cũng không đủ để biết mọi segment đã drain.

## Decision

Tạo Module `SpeechOutput` với Interface `push_delta`, `finish_input`, `cancel`, `poll` và event `SegmentReady`, `Started`, `AudioPacket`, `Drained`. `SegmentReady` mang text hiển thị nguyên bản của câu; module làm sạch bản text riêng trước khi đưa vào TTS. Module che giấu TTS provider stream và toàn bộ audio pipeline; nó không có WS sender.

`SessionActor` là producer duy nhất của outbound queue. WS writer là caller duy nhất của WebSocket send và dùng `GenerationGate` read-only để drop mọi payload turn stale. Actor cập nhật gate trước khi enqueue session-control `tts:stop` khi abort.

## Consequences

Caller chỉ học một lifecycle nhỏ thay vì timing của TTS, Opus và pacing. Cancel và ordering có Locality tại actor/writer; test có thể kiểm tra `Drained` và generation gate qua Interface thay vì chọc vào queue nội bộ.
