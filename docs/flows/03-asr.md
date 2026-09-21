# Flow 03 — Streaming ASR

## 1. Boundary

ASR Phase 3 là streaming nội bộ: `AsrProvider::open()` tạo `AsrSession`; `push_pcm()` nhận PCM 16 kHz khi người dùng đang nói và có thể trả `AsrPartial`; `finish()` drain recognizer rồi trả đúng một `AsrFinal`. Default adapter là `zipformer_sherpa` local Rust. HTTP/offline adapter tương lai có thể buffer ở `push_pcm()` rồi infer tại `finish()`, nhưng không đổi contract.

`AsrStreamLease` được lấy trước khi mở stream (`SpeechStart` ở Auto, `listen:start` ở Manual) và release sau final, cancel hoặc lỗi. `Active Turn` permit chỉ được lấy tại `SpeechEnd`/`listen:stop`, trước `finish()`; nếu không có permit, server cancel stream, release lease và bỏ turn.

## 2. Flow

```mermaid
sequenceDiagram
    participant A as SessionActor
    participant ASR as AsrProvider
    participant WS as WS Writer
    participant LLM as LLM Pipeline

    A->>ASR: open + push_pcm*(generation)
    ASR-->>A: AsrPartial(text, generation) internal only
    A->>ASR: finish() after Active Turn permit
    ASR-->>A: AsrFinal(text, generation)
    alt generation still current and text.trim() non-empty
      A->>A: commit user utterance
      A->>WS: exactly one existing stt{session_id, text}
      A->>LLM: begin_turn(text)
    else generation still current and text.trim() empty
      A->>A: CompletedSilent; release permit; return by mode
    else stale, failed, or cancelled
      A->>A: release resources; no STT
    end
```

## 3. Wire semantics và normalization

`AsrPartial` chỉ là event nội bộ có thể coalesce: không WebSocket, không dialogue và không LLM. Chỉ `AsrFinal` current-generation, non-empty mới phát đúng một message hiện có:

```json
{"session_id":"...","type":"stt","text":"xin chào"}
```

V1 không thêm `stt_partial`, `asr_partial`, `asr_final`, `vad_start`, `vad_stop` hoặc field partial/final mới. Final rỗng, ASR failure/cancel hoặc event stale không gửi `stt`.

Provider adapter chuẩn hóa model result về:

```rust
pub struct AsrResult {
    pub text: String,
    pub language: Option<String>,
    pub confidence: Option<f32>,
}
```

Core không parse JSON vendor.

## 4. Timeout và retry

V1 không automatic retry một logical ASR operation, kể cả transient connection failure. `asr.timeout_ms` bắt đầu tại `SpeechEnd`/`listen:stop` và chỉ giới hạn `finish()` tới terminal result, không tính từ `open()`. Timeout/lỗi trả về actor theo terminal path; turn tiếp theo mới là lần thử mới.
- `text.trim()` rỗng: `CompletedSilent`, không STT/LLM/TTS/dialogue, release permit; manual về Ready và auto về Listening.

## 5. Cancellation

Nếu generation bị cancel trong lúc ASR đang chạy:

- actor bump generation trước cancel, nhưng chỉ release `AsrStreamLease` sau `Cancelled` acknowledgement của worker.
- nếu response vẫn về, actor drop do generation mismatch; `Cancelled` cleanup event vẫn phải xử lý dù generation stale.
- hết cleanup grace không acknowledgement: quarantine worker và fail closed affected Voice Session.

## 6. Test contract

Mock provider cases:

- success Vietnamese final phát đúng một `stt` trước LLM.
- partial thay đổi/coalesce nhưng không gửi WebSocket, không commit dialogue, không start LLM.
- `finish()` drain recognizer trước final.
- `SpeechStart`/`listen:start` acquire `AsrStreamLease`; endpoint không có Active Turn permit cancel stream/release lease, không `finish()`, STT hay LLM.
- empty text.
- whitespace-only text -> CompletedSilent, không tạo dialogue turn.
- timeout.
- không có automatic retry khi timeout/network error.
- malformed upstream response.
- network error.
- stale generation result bị drop.
- STT JSON đúng schema trước khi LLM turn bắt đầu.
