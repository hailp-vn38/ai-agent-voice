# Flow 06 — Abort, barge-in và cancellation

## 1. Lý do

Voice UX chỉ ổn định khi mọi output có thể bị vô hiệu hóa bởi control event được phép. V1 chưa có server AEC nên không dùng microphone audio hay VAD để tự hủy khi đang Speaking.

## 2. Turn model

```rust
pub struct TurnContext {
    pub generation: u64,
    pub cancel: CancellationToken,
}
```

Mỗi turn mới:

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

    ESP->>A: abort OR listen:start
    A->>L: cancel generation N
    A->>T: cancel generation N
    A->>A: generation = N+1; update GenerationGate
    A->>W: session-control tts:stop
```

## 4. Double protection

Cancellation token giúp dừng producer sớm. GenerationGate ở writer bảo vệ packet đã vào queue khi upstream không cancel kịp.

Do đó mọi async result từ ASR/LLM/TTS nên mang generation.

## 5. Queue cleanup

Không gọi `clear()` tùy tiện trên channel dùng chung nhiều generation nếu có thể race. Tốt hơn:

- mọi payload turn có generation;
- writer drop payload stale theo GenerationGate;
- queue capacity nhỏ;
- producer cũ bị cancel.

## 6. Test contract

- abort trong LLM stream -> không có delta/TTS mới của turn cũ.
- abort khi TTS queue có packet -> packet stale không send.
- ASR response cũ về trễ -> drop.
- hai abort liên tiếp -> idempotent, không panic.
- disconnect -> root cancellation hủy mọi turn.
- manual mode: raw microphone audio khi Speaking không hủy turn.
- auto mode: VAD chỉ endpoint speech khi Listening; V1 không acoustic barge-in khi Speaking.
- provider overload/error trước `tts:start` -> cancel im lặng, telemetry và về ready/listening; sau `tts:start` -> cancel, drop stale audio rồi gửi `tts:stop`.
