#!/usr/bin/env bash
set -euo pipefail

gate_runtime="${VOICE_ONNX_RUNTIME_LIB:?Phase 5 real-model gate unavailable: set VOICE_ONNX_RUNTIME_LIB}"

if [ ! -f "$gate_runtime" ]; then
  echo "Phase 5 real-model gate unavailable: missing $gate_runtime" >&2
  exit 2
fi

scripts/test-phase5-offline-preflight.sh

VOICE_ONNX_RUNTIME_LIB="$gate_runtime" \
cargo test -q -p voice-agent-server --features real-model-gate --test phase5_reference_gate -- --nocapture
