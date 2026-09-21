#!/usr/bin/env bash
set -euo pipefail

module="${1:-}"

if [ -z "$module" ]; then
  echo "usage: $0 {ws|vad|asr|llm|tts|e2e|mcp}"
  exit 2
fi

case "$module" in
  ws)
    cargo test -p voice-agent-server --test ws_protocol -- --nocapture
    ;;
  vad)
    cargo test --test vad -- --nocapture
    ;;
  asr)
    cargo test --test asr_contract -- --nocapture
    ;;
  llm)
    cargo test --test llm_stream -- --nocapture
    ;;
  tts)
    cargo test --test tts_stream -- --nocapture
    ;;
  e2e)
    cargo test -p voice-agent-server --test ws_protocol -- --nocapture
    ;;
  mcp)
    cargo test --test device_mcp -- --nocapture
    ;;
  *)
    echo "unknown module: $module"
    exit 2
    ;;
esac
