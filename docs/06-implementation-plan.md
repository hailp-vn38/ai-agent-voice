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

- VAD state machine.
- pre-roll/end silence.
- ASR trait + first HTTP provider.
- STT response.

Exit criteria: nói vào fake/real device tạo đúng STT.

## Phase 4 — LLM + TTS streaming

Deliverables:

- OpenAI-compatible LLM stream.
- sentence segmenter.
- TTS trait + first provider.
- output resample/Opus.
- AudioPacer.
- `SpeechOutput` command/event lifecycle (`FinishInput` / `Drained`).

Exit criteria: TTS first audio xuất hiện trước khi LLM stream hoàn tất với câu đủ dài.

## Phase 5 — Interrupt correctness

Deliverables:

- generation lifecycle.
- CancellationToken.
- stale result filtering.
- `GenerationGate` ở writer.
- barge-in.

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
