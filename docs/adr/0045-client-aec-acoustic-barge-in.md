# ADR 0045 — Acoustic Barge-in client-AEC opt-in và interruption linearization

## Status

Accepted

> Supersedes ADR-0010. Supersedes riêng rule `listen:start` interruption và binary-only-Listening của ADR-0022; bổ sung scheduling của ADR-0013 nhưng không thay thế Single Writer.
> ADR-0046 thay riêng ownership tạo wire `tts:start`/`tts:stop`: actor gửi urgent `AbortTurn`, writer phân xử terminal outcome và tạo stop có điều kiện. Điều kiện Acoustic Barge-in và GenerationGate ở đây vẫn áp dụng.

## Context

Phase 3 đã có VAD/ASR streaming và `AutoPcmRetention` bounded, còn Phase 4 có streaming TTS. Giữ microphone drop toàn bộ khi `Speaking` khiến client đã AEC không thể nói chen; nhưng raw WebSocket V1 không có timestamp/reference audio để server tự làm hoặc chứng minh AEC. Đồng thời cancel producer không thu hồi được JSON/audio đã nằm trong writer queue.

## Decision

- `ClientHello.features.aec` optional/default false là Echo-safe Client Assertion, không phải server-side AEC. Acoustic Barge-in chỉ hợp lệ khi `barge_in.enabled`, `barge_in.trust_client_aec_feature`, assertion `aec=true`, và mode là `Auto` hoặc `Realtime`.
- `Manual` không acoustic barge-in. `listen:start` chỉ arm/reset VAD Capture Cycle, không cancel turn hay tăng GenerationId. `Auto` chỉ watch khi cycle đã arm; `Realtime` giữ VAD armed xuyên `Processing`/`Speaking`. Explicit `abort` luôn interrupt.
- VAD Worker Lease, VAD Capture Cycle và Conversational Turn có identity khác nhau. Event semantic phải đúng cycle hiện hành; cleanup acknowledgement stale vẫn xử lý.
- On acoustic `SpeechStart`, actor snapshot retention từ `start_sample - pre_roll` trước mọi reset; sau đó shared GenerationGate invalidate turn N, cancel token/producer N, urgent gửi đúng một `tts:stop` nếu N đã Started, rồi mở ASR turn N+1 và feed snapshot. Gate invalidation là linearization point; packet đã writer-send trước đó không recall được.
- Writer dùng lane bounded `urgent > normal control > audio`, gate mọi turn-scoped JSON/audio ngay trước admission/send. Không admission được urgent stop sau `tts:start` là session-integrity failure: root cancellation/writer shutdown fail-closed, không silent ignore hay retry vô hạn.
- Reuse/generalize `AutoPcmRetention`, overwrite oldest, với capacity `pre_roll + confirmation + vad_command_capacity * 960 + 960 + rechunk slack`; không thêm fixed 500 ms ring.

## Consequences

- Mặc định hai config Barge-in false nên rollout không tự đổi hành vi client cũ. Không assertion/trust thì microphone khi Speaking không đổi turn.
- Phase 5 phải test stale JSON lẫn audio, late VAD cycle, urgent-lane pressure, triggering PCM/pre-roll và Reference Client E2E quiet period. Real ZeroTTS/reference-client proof và firmware playback HIL là các gate riêng.
- Server-side AEC vẫn ngoài scope: nó cần transport timestamp/reference mapping và boundary riêng, không được suy diễn từ `features.aec`.
