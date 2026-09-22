# 04: Tương thích protocol và kiểm chứng end-to-end

**What to build:** Phase 3 được kiểm chứng từ góc nhìn firmware-compatible client: mọi utterance hợp lệ có một STT V1 final đúng thứ tự, không lộ partial/VAD events, và reference client chứng minh đường Opus canonical hoạt động với audio mẫu qua local VAD/ASR ở cả Manual lẫn Auto mode.

**Blocked by:** 01: Nền tảng local VAD/ASR và Manual STT; 02: Worker runtime VAD/ASR có acknowledgement; 03: Auto VAD, capacity và lifecycle session baseline; 05: VAD inference và segmentation correctness remediation; 06: Model Preparation lifecycle; 07: Compile-time provider registry.

**Status:** ready-for-agent

- [ ] WebSocket integration tests với provider giả xác nhận một STT final trước terminal state, không có message type/field partial hoặc VAD mới, và các failure/cancellation/stale path không phát STT.
- [ ] Reference client kiểm tra `docs/audio.wav` qua canonical Opus transport cho Manual và Auto với local model configuration, tách biệt rõ deterministic CI và real-model smoke.
- [ ] Bộ kiểm chứng ghi nhận rằng audio/transcript không xuất hiện trong telemetry hoặc log kiểm thử, và cập nhật tài liệu vận hành cần thiết để chạy smoke cục bộ.
- [ ] Gate cuối Phase 3: real-model Reference Client chạy canonical Opus với Manual và Auto; mỗi scenario phát exactly one existing STT, sau khi deterministic remediation 05–07 pass. Không suy ra gate này từ fake provider test.
