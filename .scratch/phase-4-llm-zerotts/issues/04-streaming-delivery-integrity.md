# 04: Streaming delivery integrity

**What to build:** Fake streaming LLM tạo Speech Segment trước EOF và SpeechOutput delivery vẫn bounded, ordered và truthful. Người dùng nhận audio sentence đầu trước khi LLM Finished; Dialogue History chỉ nhận Delivered Assistant Response sau Drained; failure/cancel không để stale audio hoặc fake playback controls.

**Blocked by:** 02: Synthetic SpeechOutput tracer bullet.

**Status:** resolved

- [x] Sentence Segmenter dùng hard/soft punctuation fixed, threshold typed và Unicode-safe hard split; segmenter là pure testable delivery policy.
- [x] Pending Speech Segment capacity fail `speech_output_backpressure` thay vì drop/overwrite text; full queue cancel LLM và không accept thêm delta.
- [x] Một generation chỉ synthesize một ordinal active, giữ playback order khi inference/pacing overlap; Drained requires FinishInput, empty pending, no active synthesis và final audio paced.
- [x] Tests cover first audio before LLM EOF, failure trước/sau Started, one stop after Started failure, GenerationGate drop stale audio, no Delivered Assistant Response on failure và normal history commit after Drained.

## Comments

- Hoàn tất 2026-09-22: `SpeechOutput` nhận incremental LLM delta qua sentence segmenter bounded; SessionActor chỉ commit assistant response sau `Drained`. Writer nhận invalidation gate theo generation để loại audio queued/stale trước playback stop.
- Xác minh: `cargo check -p voice-agent-server`, `cargo test -p voice-agent-server --lib`, `cargo test -p voice-agent-server --test speechoutput_tracer`, `git diff --check` pass. `cargo test --workspace` bị chặn bởi test untracked của ticket 05 (`zerotts_artifact_preflight.rs`) tham chiếu API ticket 05 chưa tồn tại.
- Cập nhật 2026-09-23: policy threshold/hard split của ticket gốc đã được thay bằng dấu kết câu flush ngay, dấu mềm không flush và `max_chars` chỉ là bound khẩn cấp; xem `.scratch/phase-4-llm-zerotts/spec.md` và hồi quy `Xin chào!` trong `speech_output.rs`.
