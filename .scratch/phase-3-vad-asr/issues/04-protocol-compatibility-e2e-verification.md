# 04: Tương thích protocol và kiểm chứng end-to-end

**What to build:** Phase 3 được kiểm chứng từ góc nhìn firmware-compatible client: mọi utterance hợp lệ có một STT V1 final đúng thứ tự, không lộ partial/VAD events, và reference client chứng minh đường Opus canonical hoạt động với audio mẫu qua local VAD/ASR ở cả Manual lẫn Auto mode.

**Blocked by:** 01: Nền tảng local VAD/ASR và Manual STT; 02: Worker runtime VAD/ASR có acknowledgement; 03: Auto VAD, capacity và lifecycle session baseline; 05: VAD inference và segmentation correctness remediation; 06: Model Preparation lifecycle; 07: Compile-time provider registry.

**Status:** in-progress

- [x] WebSocket integration tests với provider giả xác nhận một STT final trước terminal state, không có message type/field partial hoặc VAD mới, và các failure/cancellation/stale path không phát STT.
- [x] Reference client kiểm tra `docs/audio.wav` qua canonical Opus transport cho Manual và Auto với local model configuration, tách biệt rõ deterministic CI và real-model smoke.
- [x] Bộ kiểm chứng ghi nhận rằng audio/transcript không xuất hiện trong telemetry hoặc log kiểm thử, và cập nhật tài liệu vận hành cần thiết để chạy smoke cục bộ.
- [ ] Gate cuối Phase 3: real-model Reference Client chạy canonical Opus với Manual và Auto; mỗi scenario phát exactly one existing STT, sau khi deterministic remediation 05–07 pass. Không suy ra gate này từ fake provider test.

## Comments

- Added deterministic WebSocket E2E coverage using injected VAD/ASR providers: canonical 16 kHz Opus succeeds once in Manual and Auto; internal partial output cannot cross the wire; failed and cancelled/stale finals emit no STT.
- Reference Client now rejects duplicate STT for a single WAV replay and keeps transcript/audio out of normal smoke output. `docs/voice-reference-client.md` records the separate real-model Manual/Auto commands and privacy-safe evidence boundary.
- The real-model Phase Completion Gate remains unchecked until locally prepared artifacts and a running local server are available.
