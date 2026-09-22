# 06 — Implementation plan

## Phase 0 foundation — prerequisite của Phase 1

Deliverables:

- Cargo project.
- config loader.
- tracing.
- `/health`.
- CI fmt/clippy/test.

Exit criteria: binary start/stop sạch, config invalid fail fast.

## Phase 1 — Protocol + transport

Deliverables:

- OTA discovery.
- WS upgrade.
- Client/server message DTO.
- protocol v1 binary forwarding.
- single writer.
- SessionActor skeleton.
- bounded ingress/outbound queue và overload policy.
- Rust CLI **Reference Client** độc lập: OTA, WS headers, ClientHello, raw binary payload và protocol-fault cases.
- Rust integration tests: OTA, handshake, protocol fault/close code, control/state và binary forwarding.

Exit criteria: Reference Client hoàn tất hello và gửi/nhận binary payload; automated protocol-conformance tests pass không cần phần cứng ESP.

## Phase 2 — Audio foundation

Deliverables:

- Opus decoder/encoder.
- PCM type.
- manual listen buffer.
- audio fixtures.

Exit criteria: automated Opus round-trip và manual utterance tests pass; trước khi Phase 2 hoàn tất phải commit một Reference Client compatibility fixture raw Opus 60 ms có provenance và test decode thành đúng 960 samples.

## Phase 3 — VAD + ASR

Deliverables:

- Provider foundation: compile-time registry/factory, typed adapter config, lifecycle/load errors và contract-test harness; không dynamic plugin hoặc runtime code discovery.
- Typed provider config chọn Logical Model Identity; Model Artifact Manifest authoritative source/revision/artifact/install path/transform/checksum. Model Preparation prepare/verify/atomic-install dưới configured root, warmup trước bind và fail-fast offline khi artifact không hợp lệ.
- `VadProvider` trả `VadProbability` với `[start_sample, end_sample)`; default `silero_onnx` local Rust (`ort`) giữ recurrent state và 64-sample context, còn rechunker vẫn tạo 512-sample model input hiện thời.
- Core-owned `VadSegmenter` chỉ giữ semantic cursor/state: hysteresis, candidate onset, `min_speech_ms`, `end_silence_ms` và `max_utterance_ms` tính theo contiguous sample timeline. Actor giữ PCM retention ring bounded cho pre-roll, confirmation horizon và bounded VAD in-flight lag; provider/segmenter không sở hữu PCM.
- Streaming `AsrProvider`/`AsrSession`; default `zipformer_sherpa` local Rust với partial/final và `finish()` drain recognizer.
- Bounded VAD/ASR worker boundary, `AsrStreamLease` và `max_asr_streams`; không block Tokio executor.
- Recognition stream pin vào một ASR worker trong toàn lifetime; worker event mang session, generation và stream identity.
- `CancelRequested` chỉ release ASR slot sau `Cancelled` acknowledgement; worker không acknowledge bị quarantine và session bị ảnh hưởng fail closed.
- Ở `SpeechEnd`/`listen:stop`, lấy `Active Turn` permit trước ASR finalization; fail-fast cancel/release nếu hết permit.
- Enter `Processing` tại utterance terminal boundary để drop audio và ngăn turn song song; Phase 3 terminal ngay sau STT, CompletedSilent hoặc failure (Manual → Ready, Auto → Listening).
- Auto pin VAD worker/session xuyên Auto Listening cycle; sau mỗi utterance re-arm boundary reset recurrent state, 64-sample context, segmenter, PCM retention và VAD cursor bookkeeping trước khi re-arm.
- VAD inference/Reset/Close failure hoặc cleanup timeout quarantine worker và fail closed affected Voice Session 1011; không silent fallback Auto → Manual hoặc làm chết server.
- VAD ingress không được silently drop canonical input rồi tiếp tục segmentation: queue-full tạo VAD Stream Integrity Failure và fail closed affected Auto Voice Session, trừ khi bounded backpressure vẫn chứng minh contiguous timeline.
- `asr.timeout_ms` bắt đầu tại endpoint và chỉ giới hạn `finish()` tới terminal result; streaming Push bị giới hạn bởi max utterance, queue, cancellation và runtime failure.
- `DialogueHistory` RAM-only bounded bằng `llm.max_history_messages` (default 20); actor chỉ gọi `commit_user`, còn eviction là responsibility của history.
- Generation-tagged worker events, stale partial/final filtering và `CompletedSilent` cho final rỗng.
- `AsrPartial` internal-only; chỉ final current-generation, non-empty mới enqueue đúng một `type:"stt"` hiện có trước LLM. Không thêm wire message/field partial hoặc VAD mới.
- Không có Python sidecar hoặc HTTP ASR trong Phase 3 baseline.

Exit criteria: deterministic provider/worker/session tests pass; Silero chạy đúng 64-context runtime contract; fixture speech tạo `SpeechStart { start_sample }`/`SpeechEnd` theo contiguous sample timeline và không mất PCM từ `start_sample - pre_roll`; retention vẫn bounded khi VAD worker lag; startup Model Preparation, offline failure và compile-time registry/factory pass contract tests; Zipformer nhận PCM trong lúc người dùng nói, drain final gần endpoint; stale final không commit sau cancel; auto/manual giải phóng đúng cả `AsrStreamLease` và `Active Turn` permit. Gate cuối riêng bắt buộc real-model Reference Client E2E qua canonical Opus: Manual và Auto mỗi scenario phát exactly one STT. Không được suy ra gate này từ fake/provider tests.

## Phase 4 — LLM + TTS streaming

Deliverables:

- OpenAI-compatible LLM stream.
- sentence segmenter.
- TTS trait + first provider.
- output resample/Opus.
- AudioPacer.
- `SpeechOutput` command/event lifecycle (`FinishInput` / `Drained`).

Exit criteria: TTS first audio xuất hiện trước khi LLM stream hoàn tất với câu đủ dài.

## Phase 5 — Explicit interruption correctness

Deliverables:

- generation lifecycle.
- CancellationToken.
- stale result filtering.
- `GenerationGate` ở writer.
- `abort` và `listen:start` interrupt explicit.
- regression bảo đảm VAD/microphone không acoustic interrupt khi `Speaking`.

Exit criteria: không có stale audio sau abort trong stress test.

## Phase 6 — Device MCP

Deliverables:

- initialize.
- tools/list pagination.
- tools/call + correlation.
- LLM tool schema integration.

Exit criteria: voice command gọi thành công một tool thật trên ESP32.

## Phase 7 — Hardening

Deliverables:

- provider timeout/error mapping.
- idle timeout.
- structured metrics.
- graceful shutdown.
- Docker/systemd docs.

Exit criteria: soak test nhiều giờ không tăng queue/RAM bất thường.

## PR/commit boundary khuyến nghị

Không làm một PR khổng lồ. Mỗi phase có thể chia 2–4 PR nhỏ theo protocol/core/provider/test.
