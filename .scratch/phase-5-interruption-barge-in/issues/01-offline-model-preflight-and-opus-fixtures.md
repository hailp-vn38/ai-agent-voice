# 01: Offline model preflight và fixture Opus Phase 5

**What to build:** Một preflight offline, tái lập được, xác nhận model VAD, ASR và ZeroTTS đã pin cùng ONNX Runtime cục bộ có thể được dùng bởi các gate Phase 5; đồng thời cung cấp fixture Opus uplink canonical cho Rust Reference Client và E2E mà không cần LLM mạng hay phần cứng.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Preflight xác minh manifest, acknowledgement, artifact cần thiết, ONNX Runtime và canonical audio fixture trước mọi gate Phase 5; thiếu/corrupt phải fail rõ ràng trước test flow.
- [x] Fixture tạo hoặc kiểm chứng raw Opus 16 kHz mono 60 ms có speech/silence đủ cho VAD onset, retention và utterance B, không chứa audio người dùng.
- [x] Gate chạy được offline với model đã cài, không yêu cầu OpenAI/API key; kết quả phân biệt pass, unavailable và fail.

## Comments

- Hoàn tất 2026-09-23: `scripts/test-phase5-offline-preflight.sh` buộc Model Preparation offline, kiểm tra acknowledgement/checksum cho Silero VAD, Zipformer ASR và ZeroTTS, nạp đúng ONNX Runtime đã cấu hình, rồi kiểm tra raw Opus fixture qua decoder 16 kHz mono/960 samples. In `pass` khi sẵn sàng, `unavailable` (exit 2) khi thiếu model/runtime và `fail` (exit 1) cho checksum/config/fixture sai.
- Fixture nằm tại `crates/voice-reference-client/tests/fixtures/phase5-uplink-*.opus`; generator tổng hợp tone xác định, không dùng audio người dùng. Chuỗi silence/speech A/silence/speech B/silence là input cho onset, retention và utterance B của ticket sau.
