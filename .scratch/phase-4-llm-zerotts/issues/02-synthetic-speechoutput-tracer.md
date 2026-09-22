# 02: Synthetic SpeechOutput tracer bullet

**What to build:** Sau một AsrFinal non-empty hiện tại, Voice Protocol Client nhận được một Generated Assistant Response synthetic hoàn chỉnh: `tts:start`, canonical Opus audio và đúng một `tts:stop`. Slice dùng fake LLM/TTS nhưng đi qua SessionActor, SpeechOutput, canonical audio pipeline, AudioPacer và WS writer như đường production.

**Blocked by:** 01: Mở rộng typed provider foundation Phase 4.

**Status:** resolved

- [x] AsrFinal current-generation non-empty bắt đầu một Conversational Turn delivery; audio hợp lệ chỉ xuất hiện sau `tts:start` và normal completion chỉ stop sau final audio paced.
- [x] SpeechOutput sở hữu Submit/FinishInput/Cancel và SessionActor vẫn là outbound producer duy nhất; fake provider không gửi WS trực tiếp.
- [x] Public application/router tests quan sát Voice Session phase, client-visible control/audio order, canonical 24 kHz Opus và release Active Turn; Phase 3 STT tests vẫn xanh.

## Comments

- Hoàn tất 2026-09-22: tracer dùng fake LLM/TTS qua `SessionActor` và `SpeechOutput`; PCM 48 kHz được đưa qua resample, canonical Opus 24 kHz và pacer trước WS writer. Test actor và router xác nhận `stt` -> `tts:start` -> binary Opus decode 1.440 samples -> đúng một `tts:stop`, sau đó turn về Ready.
- Xác minh: `cargo test -p voice-agent-server --test speechoutput_tracer`, `cargo test -p voice-agent-server --test manual_stt --test protocol_e2e --test config_audio`, `cargo check -p voice-agent-server` pass. `cargo test --workspace` đang không compile vì file ngoài ticket `tests/zerotts_artifact_preflight.rs` tham chiếu API ticket 05 chưa tồn tại; không sửa hoặc format file đó trong ticket 02.
