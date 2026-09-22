# 01: Mở rộng typed provider foundation Phase 4

**What to build:** Application có thể khởi tạo LLM và TTS fake qua Typed Provider Configuration, Provider Registry compile-time, Provider Factory, ProviderSet và application runtime mà không làm thay đổi behavior VAD/ASR Phase 3. Đây là seam public để các ticket sau inject deterministic provider và để production chọn adapter built-in tại startup.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Typed config và validation biểu đạt OpenAI LLM, ZeroTTS TTS, LLM/TTS capacity, TTS worker configuration và SpeechOutput bounds; config sai fail trước bind.
- [x] Provider Registry/loader/ProviderSet/AppState mở rộng LLM/TTS theo factory compile-time, không dynamic discovery hoặc direct model path; VAD/ASR behavior hiện hữu không regression.
- [x] Tests ở application/config/provider seam inject fake LLM/TTS và xác nhận typed selection, uncompiled adapter reject, capacity mismatch reject và không lộ API key.

## Comments

- Hoàn tất 2026-09-22: thêm typed `openai`/`zerotts_onnx` foundation, factory registry compile-time và ProviderSet injection seam. LLM network streaming và ZeroTTS model preparation/synthesis vẫn thuộc ticket 03/05/06.
- Xác minh: `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, và `git diff --check` đều pass; một test smoke Reference Client cần local model artifact được đánh dấu ignored như trước.
