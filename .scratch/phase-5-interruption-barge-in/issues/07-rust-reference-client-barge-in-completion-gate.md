# 07: Rust Reference Client barge-in completion gate

**What to build:** Rust Reference Client tự thực hiện flow hai utterance với audio canonical: đưa A tới TTS Started, gửi B trong lúc playback, và xác minh public Phase 5 contract bằng model cục bộ thay vì chỉ dựa fake client hoặc phần cứng.

**Blocked by:** 06 — Acoustic Barge-in trong Voice Session.

**Status:** resolved

- [x] Reference Client có scenario Barge-in strict: Hello AEC, Auto/Realtime, uplink Opus A/B, correlated `tts:start`/`tts:stop`, no reconnect, timeout hữu hạn và quiet period sau stop.
- [x] Gate bắt buộc dùng ZeroTTS và ONNX Runtime đã preflight; fake deterministic LLM loại bỏ dependency mạng. Kết quả chứng minh không stale A sau boundary và B tới STT rồi TTS trên cùng socket.
- [x] Tách rõ evidence: Reference Client/real-model pass là Phase 5 completion gate; firmware playback, microphone vật lý và chất lượng AEC thực là HIL riêng, không được suy diễn pass.

## Comments

- Hoàn tất 2026-09-23: `run_barge_in` là seam công khai của Rust Reference Client. Nó assert `features.aec=true`, dùng fixture uplink Opus canonical A/B, chờ `tts:start` A, rồi xác nhận đúng một interruption `tts:stop`, không audio qua boundary trước `tts:start` B, STT B, Opus B canonical và quiet period sau terminal stop. Gate chạy cả Auto lẫn Realtime trên cùng ZeroTTS provider đã được warm/load với ONNX Runtime cục bộ; LLM, VAD và ASR deterministic chỉ làm nguồn input hợp đồng ổn định, không dùng mạng.
- Xác minh 2026-09-23: `scripts/test-phase5-offline-preflight.sh` pass; `VOICE_ONNX_RUNTIME_LIB=<verified dylib> scripts/test-phase5-reference-gate.sh` pass; `cargo fmt --all -- --check`, `cargo check -p voice-agent-server`, `git diff --check`, và `cargo test --workspace -q` pass.
- Boundary HIL: gate này không chứng minh firmware playback, microphone vật lý hoặc hiệu quả AEC thực; các bằng chứng đó vẫn phải thu trên HIL Reference Profile riêng.
