#!/usr/bin/env bash
set -euo pipefail

if [ "${RUN_LIVE_API_TESTS:-0}" != "1" ]; then
  echo "Set RUN_LIVE_API_TESTS=1 to run live provider tests."
  exit 2
fi

cargo test live_provider -- --ignored --nocapture
