# Testing 00 — Chiến lược test tổng thể

## 1. Mục tiêu

Test phải giúp sửa một module mà không cần thiết bị thật hoặc API thật ở vòng lặp phát triển hằng ngày.

## 2. Test pyramid

```text
          E2E với ESP32 / live providers
             contract tests
         integration tests với mocks
              unit tests
```

Mặc định CI chỉ chạy unit + integration + mock contract tests.

Live API tests phải opt-in bằng environment flag.

## 3. Test categories

### Unit

Pure logic:

- serde protocol DTO.
- sentence segmentation.
- VAD state aggregation.
- generation filtering.
- tool-name sanitization.
- dialogue trimming.

### Integration

- WebSocket reader/writer + actor.
- actor + mock providers.
- TTS -> Opus -> pacer.
- MCP correlation.
- `SpeechOutput` lifecycle: nhiều segment, `FinishInput`, `Drained`, cancel.
- overload policy của ingress/outbound queue.

### Contract

Mỗi provider adapter có fixture response đã capture hoặc mock HTTP server để kiểm tra mapping vendor -> domain.

### Live

Chỉ chạy khi explicit:

```text
RUN_LIVE_API_TESTS=1
```

## 4. Test naming

```text
tests/ws_protocol.rs
tests/vad.rs
tests/asr_contract.rs
tests/llm_stream.rs
tests/tts_stream.rs
tests/e2e_voice.rs
```

## 5. Golden fixtures

`tests/fixtures/` nên chứa:

- client hello JSON.
- listen start/stop/detect JSON.
- MCP initialize/tools responses.
- một vài Opus frames mẫu.
- PCM silence/speech fixtures nhỏ.
- mock SSE stream của LLM.
- mock TTS audio response.
- ASR WAV multipart request + JSON `{ "text": ... }` response.
- LLM Chat Completions SSE: text-only, fragmented tool call, tool-error follow-up và multi-tool delta.
- TTS speech JSON request + WAV response.

Không commit audio lớn.

Với codec Phase 2, fixture audio gồm PCM synthetic 960 samples và ít nhất một raw uplink Opus 60 ms capture từ Reference Client độc lập phù hợp Firmware Baseline, kèm metadata provenance (release, commit, protocol, sample rate, channels, duration). Round-trip chỉ assert encode không rỗng, decode thành đúng 960 samples và sanity audio; không so byte output Opus hay PCM lossy sample-for-sample. Synthetic tests là automated implementation gate; compatibility fixture và decode test của nó là gate bắt buộc trước khi Phase 2 được đánh dấu hoàn tất.

## 6. Determinism

Test pacing/time không nên phụ thuộc sleep dài. Dùng Tokio paused time (`tokio::time::pause/advance`) khi có thể.

Provider mock phải deterministic.

## Privacy telemetry

Test logger/telemetry phải chứng minh không có raw PCM/Opus, transcript, prompt/response, generated text, MCP payload, Device/Client ID nguyên bản, Authorization/token/API key hay body OTA/provider. Test chỉ chấp nhận metadata vận hành và Trace Session ID ngẫu nhiên.
