# Flow 06 — Abort, barge-in và cancellation

## 1. Lý do

Voice UX chỉ ổn định khi mọi output có thể bị vô hiệu hóa bởi control event được phép. Phase 5 thêm Acoustic Barge-in client-AEC opt-in, không phải server-side AEC: `features.aec=true` chỉ có hiệu lực khi server bật cả `barge_in.enabled` và `barge_in.trust_client_aec_feature`.

## 2. Turn model

```rust
pub struct TurnContext {
    pub generation: u64,
    pub cancel: CancellationToken,
}
```

Mỗi turn semantic mới:

```text
cancel old token
current_generation += 1
create new token
```

## 3. Abort flow

```mermaid
sequenceDiagram
    participant ESP as ESP32/User
    participant A as SessionActor
    participant L as LLM
    participant T as TTS
    participant P as Pacer
    participant W as WS Writer

    ESP->>A: abort OR AEC-safe VAD SpeechStart
    A->>L: cancel generation N
    A->>T: cancel generation N
    A->>A: snapshot pre-roll; invalidate GenerationGate N; generation = N+1
    A->>W: urgent session-control tts:stop
```

## 4. Double protection

Cancellation token giúp dừng producer sớm. GenerationGate shared ở writer bảo vệ JSON lẫn packet đã vào queue khi upstream không cancel kịp. Gate invalidation là linearization point: writer không admission thêm turn payload N sau point đó; frame đã được gửi trước point không thể recall.

Do đó mọi async result từ ASR/LLM/TTS nên mang generation.

## 5. Queue cleanup

Không gọi `clear()` tùy tiện trên channel dùng chung nhiều generation nếu có thể race. Tốt hơn:

- mọi payload turn có generation;
- writer drop mọi payload turn stale theo GenerationGate;
- queue capacity nhỏ;
- producer cũ bị cancel.

Sau `tts:start`, interrupt stop đi qua urgent lane bounded `urgent > normal control > audio`. Nếu không admission được, session fail-closed qua root cancellation/writer shutdown escape path; không retry vô hạn hay continue playback state mơ hồ.

## 6. Test contract

- abort trong LLM stream -> không có delta/TTS mới của turn cũ.
- `listen:start` khi Speaking chỉ arm/reset VAD Capture Cycle, không invalidate/cancel turn.
- abort khi TTS queue có packet -> JSON/audio stale không send.
- ASR response cũ về trễ -> drop.
- hai abort liên tiếp -> idempotent, không panic.
- disconnect -> root cancellation hủy mọi turn.
- manual/no-AEC: microphone khi Speaking không hủy turn.
- Auto đã arm hoặc Realtime giữ armed + AEC trusted: VAD `SpeechStart` snapshot retention trước reset rồi tạo đúng một acoustic interruption; triggering PCM/pre-roll phải đến ASR turn mới.
- provider overload/error trước `tts:start` -> cancel im lặng, telemetry và về ready/listening; sau `tts:start` -> cancel, drop stale audio rồi gửi `tts:stop`.
