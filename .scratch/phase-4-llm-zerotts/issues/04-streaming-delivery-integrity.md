# 04: Streaming delivery integrity

**What to build:** Fake streaming LLM tạo Speech Segment trước EOF và SpeechOutput delivery vẫn bounded, ordered và truthful. Người dùng nhận audio sentence đầu trước khi LLM Finished; Dialogue History chỉ nhận Delivered Assistant Response sau Drained; failure/cancel không để stale audio hoặc fake playback controls.

**Blocked by:** 02: Synthetic SpeechOutput tracer bullet.

**Status:** claimed

- [ ] Sentence Segmenter dùng hard/soft punctuation fixed, threshold typed và Unicode-safe hard split; segmenter là pure testable delivery policy.
- [ ] Pending Speech Segment capacity fail `speech_output_backpressure` thay vì drop/overwrite text; full queue cancel LLM và không accept thêm delta.
- [ ] Một generation chỉ synthesize một ordinal active, giữ playback order khi inference/pacing overlap; Drained requires FinishInput, empty pending, no active synthesis và final audio paced.
- [ ] Tests cover first audio before LLM EOF, failure trước/sau Started, one stop after Started failure, GenerationGate drop stale audio, no Delivered Assistant Response on failure và normal history commit after Drained.
