# 06: ZeroTTS synthesis-core parity

**What to build:** ZeroTTS native Rust core loads the verified pinned `maichi` pack and produces deterministic audio-code frames from a non-user text fixture without any WebSocket, SessionActor, Opus, codec PCM or mutable Voice Session ownership. Đây là provider-core slice chứng minh tokenizer, voice latent và ba graph TTS trước codec streaming.

**Blocked by:** 05: ZeroTTS artifact provenance và startup preflight.

**Status:** resolved

- [x] Parse the actual `maichi` NPZ latent in native Rust (the ticket authorizes direct ndarray/NPZ reader dependencies), validate exact `(1, n_voice_queries, d_model)` shape against config and all declared graph input/output names, dtypes and ranks. Build/load failure for any pinned role fails before inference; no Python helper, HTTP service, guessed tensor layout or hand-written latent is allowed.
- [x] Run the installed pinned `text_encoder`, `prefix_step` and `local_frame_decode` graphs through the existing configured ONNX Runtime. One synthesis operation owns all mutable KV/cache, seen-mask, frame index and deterministic random-draw state; shared engine resources hold only immutable tokenizer/config/voice/graph material. State is reset/dropped before the next operation and cannot cross a Speech Segment or Voice Session.
- [x] Encode a checked-in non-user parity fixture with NFC/whitespace normalization, pinned token IDs, fixed draws and an explicit bounded frame limit. Assert artifact-derived checkpoints for token IDs, first text-state checksum and first N code frames, including position `n_voice_queries + 1 + frame_index`, per-codebook seen-mask update and retaining the frame that first reports EOA. A mismatch fails the test; the fixture contains no user text, key or audio.
- [x] Provide one explicit local-model acceptance command that receives the verified installed pack and ONNX Runtime path. It runs the three real graphs and the parity fixture; no `#[ignore]`, missing-artifact skip or synthetic JSON test can satisfy this criterion. If CI is expected to close this ticket, it must provision those pinned artifacts/runtime first.
- [x] Keep this slice strictly pre-codec: it must not claim PCM, codec external-data validation, startup warmup, TtsWorkerRuntime, SpeechOutput, Opus or Reference Client delivery. Those are Ticket 07/08 gates.

## Comments

- Audit 2026-09-22: the previous ticket had only a broad "golden/parity" statement. The current implementation tests a hand-made tokenizer/config document and does not load a voice NPZ or execute an ONNX graph, so it does not satisfy this ticket yet. The acceptance gate above makes the required dependency, pinned fixture, real-runtime command and scope boundary explicit before implementation starts.

- Hoàn tất 2026-09-22: `ZeroTtsContract` load trực tiếp `voice.npz`, kiểm tra shape/finite và exact graph I/O contract; mỗi `synthesize_codes` tạo `ZeroTtsOperation` riêng với ONNX sessions, KV/valid mask/seen-mask/frame index/draw state chỉ sống trong operation. Fixture không-user kiểm checkpoint tokenizer, checksum, bốn frame đầu, positions 11-14, seen-mask counts và EOA frame 30 được giữ trong 31 frame.
- Xác minh local-model thật (không ignored/skip):
  `ZEROTTS_CONFIG="$PWD/models/zerotts/config.json" ZEROTTS_TOKENIZER="$PWD/models/zerotts/tokenizer.json" ZEROTTS_MAICHI_VOICE="$PWD/models/zerotts/voices/maichi/voice.npz" ZEROTTS_TEXT_ENCODER="$PWD/models/zerotts/onnx/text_encoder.onnx" ZEROTTS_PREFIX_STEP="$PWD/models/zerotts/onnx/prefix_step.onnx" ZEROTTS_LOCAL_FRAME_DECODE="$PWD/models/zerotts/onnx/local_frame_decode.onnx" VOICE_ONNX_RUNTIME_LIB=/Users/lamphuchai/.cache/uv/archive-v0/nPVYFfIRm377yvKVxcwxu/lib/python3.10/site-packages/onnxruntime/capi/libonnxruntime.1.23.2.dylib cargo run -q -p voice-agent-server --bin zerotts-core-check`
  in `ZeroTTS parity accepted: 31 frames, EOA frame 30`; command chạy hai operation liên tiếp và fail nếu state cũ rò qua operation sau. `cargo fmt --check`, `cargo check -p voice-agent-server`, `cargo test --workspace` và `git diff --check` pass. `scripts/test-all.sh` còn fail vì lint ngoài scope ở `workers/llm.rs` (`result_unit_err`) và `session/speech_output.rs` (`items_after_test_module`); không có lint Phase 06.
