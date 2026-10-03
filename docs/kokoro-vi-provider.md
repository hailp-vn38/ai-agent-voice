# Kokoro Vietnamese ONNX provider

`kokoro_vi_onnx` is a local, non-streaming TTS adapter for the pinned
`contextboxai/Kokoro-Vietnamese` ONNX graph. It returns one complete 24 kHz
mono PCM segment; `SpeechOutput` continues to own Opus encoding, pacing, and
the Voice WebSocket lifecycle.

## Deployment assets

The upstream revision is `9f210d622209fcc216fe2ac6159fed2ff381cb8a` under
Apache-2.0. Its ONNX graph inputs are `input_ids`, `ref_s`, and `speed`; the
provider emits the graph's `waveform` output.

The server must receive three Model Artifact Manifest roles:

- `model`: `kokoro_vi.onnx`;
- `config`: upstream `config.json` vocabulary;
- `voicepack_<voice>`: a prepared binary for each selectable voice; for
  example, `voicepack_diem_trinh`.

`voicepack` has the following little-endian format. It is intentionally not a
PyTorch file, so the server never needs LibTorch, `torch`, or a Python asset
converter.

```text
KOVI_VOICEPACK_V1
u32 rows
u32 channels = 1
u32 dimension = 256
rows * 1 * 256 float32 values
```

Select the row `min(phoneme_count - 1, rows - 1)`. Prepare and checksum this
file outside the server startup workflow, then publish it through the
deployment's artifact source and manifest. The current upstream repository
only publishes `.pt` voicepacks; it does not publish this production binary.

For automatic first-start acquisition, map the prepared manifest source to an
HTTP(S) URL serving that exact binary. Its SHA-256 must match the manifest; a
source URL does not change model identity or bypass verification.

```toml
[deployment.models.sources]
"prepared://deployment/kokoro-vi/voicepacks/diem_trinh.bin" = "https://your-artifact-host/diem_trinh.bin"
```

No mapping is needed when the checksum-matching voicepack is already installed.
Offline Model Preparation never contacts the URL. The server does not download
or execute G2P programs or deserialize upstream `.pt` voicepacks.

## G2P sidecar

Vietnamese text is converted by a persistent deployment-owned JSONL program,
normally a thin `vig2p` wrapper. The executable path belongs only in
`runtime.kokoro_vi.g2p_executable`, not in provider config or Admin data.

```json
{"id":1,"text":"Xin chào."}
{"id":1,"phonemes":"...","error":null}
```

Each native TTS worker owns one child process and one ONNX session. The child
is killed with that worker; it is never shared across Voice Sessions.

For a local macOS development runtime, create a Python 3.12 virtual environment
at `runtime/kokoro-vi/.venv`, install `vig2p` and `onnxruntime`, place its arm64
`libonnxruntime.dylib` at `runtime/onnxruntime/libonnxruntime.dylib`, and make
`runtime/kokoro-vi/kokoro_vi_g2p` executable. These deployment assets are
ignored by Git.

## Configuration

```toml
[providers.tts.instances.kokoro_vi]
adapter = "kokoro_vi_onnx"
model = "kokoro_vi_contextbox"
num_threads = 1
voice = "diem_trinh"
language = "vi-VN"
speed_percent = 100

[provider_defaults]
# Select `kokoro_vi` only after the declared deployment assets are installed.
tts = "kokoro_vi"

[runtime.kokoro_vi]
g2p_executable = "runtime/kokoro-vi/kokoro_vi_g2p"
```

Supported voices are `diem_trinh`, `hung_thinh`, `mai_linh`, `mai_loan`,
`manh_dung`, `my_yen`, `ngoc_huyen`, `phat_tai`, `thanh_dat`, `thuc_trinh`,
`tuan_ngoc`, `storyvert`, `duc_an`, and `duc_duy`. `speed_percent` is bounded
to 50–200, avoiding a new floating-point Provider configuration type.

Run the local real-model gate after asset preparation:

```bash
cargo test -p voice-agent-server \
  providers::tts::kokoro_vi::real_model_tests::loads_local_contextbox_model_and_synthesizes_24khz_pcm \
  -- --ignored --exact
```
