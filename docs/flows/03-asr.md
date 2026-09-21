# Flow 03 — ASR

## 1. Boundary

ASR chỉ nhận một `AudioUtterance` đã hoàn tất ở V1.

```rust
pub struct AsrRequest {
    pub generation: u64,
    pub pcm: bytes::Bytes,
    pub sample_rate: u32,
    pub channels: u8,
}
```

Adapter concrete V1 là `openai_transcription_v1`: `POST {base_url}/v1/audio/transcriptions`, multipart `file` chứa WAV PCM16/16 kHz/mono, `model`, `response_format=json` và optional `language`; response tối thiểu là `{ "text": "..." }`. Adapter chịu trách nhiệm WAV encode boundary, actor chỉ nhận `AsrResult`.

## 2. Flow

```mermaid
sequenceDiagram
    participant A as SessionActor
    participant ASR as AsrProvider
    participant WS as WS Writer
    participant LLM as LLM Pipeline

    A->>ASR: transcribe(utterance, generation)
    ASR-->>A: AsrFinal(text, generation)
    alt generation still current and text.trim() non-empty
      A->>WS: stt{text}
      A->>LLM: begin_turn(text)
    else generation still current and text.trim() empty
      A->>A: CompletedSilent; release permit; return by mode
    else stale or empty
      A->>A: drop result
    end
```

## 3. Normalization

Provider adapter chịu trách nhiệm chuyển response riêng của vendor về:

```rust
pub struct AsrResult {
    pub text: String,
    pub language: Option<String>,
    pub confidence: Option<f32>,
}
```

Core không parse JSON vendor.

## 4. Timeout và retry

V1 không automatic retry một logical ASR operation, kể cả transient connection failure. Timeout/lỗi trả về actor theo terminal path; turn tiếp theo mới là lần thử mới.
- `text.trim()` rỗng: `CompletedSilent`, không STT/LLM/TTS/dialogue, release permit; manual về Ready và auto về Listening.

## 5. Streaming ASR sau V1

Trait có thể được mở rộng bằng một trait riêng thay vì phá API batch:

```text
BatchAsrProvider
StreamingAsrProvider
```

Không ép mọi provider phải streaming.

## 6. Cancellation

Nếu generation bị cancel trong lúc ASR đang chạy:

- cancel request nếu client hỗ trợ.
- nếu response vẫn về, actor drop do generation mismatch.

## 7. Test contract

Mock provider cases:

- success Vietnamese text.
- empty text.
- whitespace-only text -> CompletedSilent, không tạo dialogue turn.
- timeout.
- không có automatic retry khi timeout/network error.
- malformed upstream response.
- network error.
- stale generation result bị drop.
- STT JSON đúng schema trước khi LLM turn bắt đầu.
