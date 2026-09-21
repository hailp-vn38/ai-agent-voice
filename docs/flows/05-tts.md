# Flow 05 — TTS, Opus và audio pacing

## 1. Input

`SpeechOutput` nhận **speakable text segment**, không nhận toàn bộ response bắt buộc. TTS provider là implementation bên trong Module này.

```rust
pub struct TtsRequest {
    pub generation: u64,
    pub text: String,
    pub voice: String,
    pub output_sample_rate: u32,
}
```

Adapter concrete V1 là `openai_speech_v1`: `POST {base_url}/v1/audio/speech` với JSON `model`, `input`, `voice`, `response_format:"wav"`. WAV response được decode, normalize/resample về PCM16 24 kHz mono rồi Opus encode; actor không thấy wire format provider.

## 2. Flow

```mermaid
flowchart LR
    SEG[Text segment] --> SO[SpeechOutput]
    SO --> TTS[TTS Provider]
    TTS --> PCM[PCM/Audio stream]
    PCM --> RS[Normalize/Resample]
    RS --> ENC[Opus encoder 60 ms]
    ENC --> Q[bounded audio queue]
    Q --> P[AudioPacer]
    P --> EV[Started/Sentence/Packet/Drained event]
    EV --> A[SessionActor]
    A --> WS[WS Writer]
```

## 3. TTS state messages

Chỉ khi AudioPacket hợp lệ đầu tiên đã sẵn sàng, actor tạo:

```text
tts:start
```

Mỗi segment có thể tạo:

```text
tts:sentence_start + text
```

Chỉ sau `FinishInput` và event `Drained`:

```text
tts:stop
```

## 4. Output audio

Output sample rate là config riêng, ví dụ 24 kHz mono. Không dùng input microphone rate làm output rate một cách mặc định.

Opus encoder đóng frame đúng `frame_ms` (khuyến nghị 60 ms tương thích firmware reference).

## 5. AudioPacer

Pacer đảm bảo packet xuống ESP32 gần tốc độ playback.

```text
prebuffer N frames -> send nhanh
sau đó -> ~frame_ms giữa các packet
```

`prebuffer_frames` phải configurable và nhỏ.

## 6. Segment lifecycle và backpressure

Mỗi segment có `ordinal` tăng dần. Actor gửi `FinishInput` khi LLM đã flush segment cuối. `SpeechOutput` chỉ phát `Drained` khi đã xử lý xong toàn bộ ordinal đã nhận và packet cuối đã qua pacer; `SegmentFinished` không đủ để gửi `tts:stop`.

TTS producer bị chặn khi bounded queue đầy. Không tạo unbounded audio queue.

## 7. Cancellation

Khi abort:

1. cancel TTS provider stream.
2. cancel producer và đánh dấu generation cũ stale; không clear shared channel.
3. pacer kiểm tra generation trước send.
4. actor cập nhật `GenerationGate` rồi gửi `tts:stop` session control.

V1 không automatic retry logical TTS operation sau timeout/lỗi.

## 8. Test contract

- input text -> audio chunks theo đúng order.
- `tts:start` chỉ đến sau first valid AudioPacket và trước packet đó trên wire.
- non-empty input + provider completion + zero valid AudioPacket -> `Failed(tts_empty_audio)`; không `tts:start`/`tts:stop` nếu start chưa gửi.
- resample output đúng target rate.
- Opus packets decode lại thành audio hợp lệ.
- pacer không burst toàn bộ sau prebuffer.
- bounded queue tạo backpressure.
- abort không cho packet generation cũ đi tiếp.
- `tts:stop` sau audio cuối, không trước.
- nhiều segment hoàn tất không theo thời điểm synthesize -> chỉ một `Drained` sau `FinishInput`.
