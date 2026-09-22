#!/usr/bin/env bash
set -euo pipefail

gate_root="${VOICE_PHASE4_MODEL_ROOT:-$PWD/models/zerotts}"
gate_runtime="${VOICE_ONNX_RUNTIME_LIB:?Phase 4 real-model gate unavailable: set VOICE_ONNX_RUNTIME_LIB}"
gate_packet="$(mktemp -t phase4-downlink-opus.XXXXXX)"
trap 'rm -f "$gate_packet"' EXIT

for gate_file in \
  "$gate_root/config.json" \
  "$gate_root/tokenizer.json" \
  "$gate_root/voices/maichi/voice.npz" \
  "$gate_root/onnx/text_encoder.onnx" \
  "$gate_root/onnx/prefix_step.onnx" \
  "$gate_root/onnx/local_frame_decode.onnx" \
  "$gate_root/onnx/codec/moss_audio_tokenizer_decode_full.onnx" \
  "$gate_root/onnx/codec/moss_audio_tokenizer_decode_step.onnx" \
  "$gate_root/onnx/codec/moss_audio_tokenizer_decode_shared.data" \
  "$gate_root/onnx/codec/codec_browser_onnx_meta.json" \
  "$gate_runtime"; do
  if [ ! -f "$gate_file" ]; then
    echo "Phase 4 real-model gate unavailable: missing $gate_file" >&2
    exit 2
  fi
done

ZEROTTS_CONFIG="$gate_root/config.json" \
ZEROTTS_TOKENIZER="$gate_root/tokenizer.json" \
ZEROTTS_MAICHI_VOICE="$gate_root/voices/maichi/voice.npz" \
ZEROTTS_TEXT_ENCODER="$gate_root/onnx/text_encoder.onnx" \
ZEROTTS_PREFIX_STEP="$gate_root/onnx/prefix_step.onnx" \
ZEROTTS_LOCAL_FRAME_DECODE="$gate_root/onnx/local_frame_decode.onnx" \
ZEROTTS_CODEC_DECODE_FULL="$gate_root/onnx/codec/moss_audio_tokenizer_decode_full.onnx" \
ZEROTTS_CODEC_DECODE_STEP="$gate_root/onnx/codec/moss_audio_tokenizer_decode_step.onnx" \
ZEROTTS_CODEC_SHARED_DATA="$gate_root/onnx/codec/moss_audio_tokenizer_decode_shared.data" \
ZEROTTS_CODEC_METADATA="$gate_root/onnx/codec/codec_browser_onnx_meta.json" \
VOICE_ONNX_RUNTIME_LIB="$gate_runtime" \
ZEROTTS_DOWNLINK_OPUS_PATH="$gate_packet" \
cargo run -q -p voice-agent-server --bin zerotts-core-check

cargo run -q -p voice-reference-client -- --ota http://127.0.0.1/unused decode-downlink-opus "$gate_packet"

VOICE_ONNX_RUNTIME_LIB="$gate_runtime" \
cargo test -q -p voice-agent-server --features real-model-gate --test phase4_reference_gate -- --nocapture
