# Native VieNeu validation

`vieneu_native` uses the pinned VieNeu v3 Turbo FP32 graphs, Rust SEA-G2P,
and the pinned MOSS streaming decoder. Production inference starts no Python
process and calls no remote TTS service. Python was used only to generate
test oracles and run the external HTTP check.

The local `config.toml` keeps ChillAudio as its default. The native instance is
enabled with `voice = "hai_dang"`, `language = "vi-VN"`, `preload = true`,
two ONNX threads, and a 2 GiB reservation for one retained replica. The initial
real-worker check measured 1,230,424 KiB peak process RSS. Requalify the
reservation when changing model revisions, execution settings, or workloads.

Validation passed: 697 workspace tests, Clippy with all targets/features and
`-D warnings`, formatting, both real-model gates, and the production HTTP check.

Validation artifacts:

WAV files and logs are generated locally and are not committed.

- `real-model.json`: initial real-worker timing and peak RSS.
- `native.wav`: direct native worker output for Hải Đăng and Mai Anh.
- `server-check.json`: production startup and managed HTTP diagnostic results.
- `server-hai_dang.wav`, `server-mai_anh.wav`: audio returned by the server.
- `workspace-tests.log`: workspace test results.

The real-worker gate checks twenty normalized phoneme/token sequences against
the recorded Python SEA-G2P 0.9.1 oracle, streaming, two voice selections,
pre-cancellation, mid-stream cancellation, callback failure, and reuse/reset.
The ignored library gate compares speaker projection, reference embeddings,
and incremental codec PCM/reset against NumPy/ONNX Runtime using the pinned
model and codec files.

```sh
cargo test -p voice-agent-server --test vieneu_native -- --ignored --nocapture --test-threads=1
cargo test -p voice-agent-server --lib embeddings_and_streaming_codec_match_pinned_numpy_ort_oracle -- --ignored --nocapture
python3 .scratch/vieneu-native/server-check.py
```

The HTTP check starts and stops its own production process with a temporary
SQLite database and nonce-bound loopback listener. It makes no external LLM
request and does not modify the normal deployment database.

Debug HTTP synthesis measured roughly 2.5–2.7 RTF while workspace compilation
was running. Release performance and microphone/WebSocket playback have not
been qualified. Voice cloning, emotion cues, and INT8 are outside this change.
