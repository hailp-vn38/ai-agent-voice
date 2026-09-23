#!/usr/bin/env bash
set -euo pipefail

# This gate performs only local reads; it never acquires artifacts or calls an LLM.
cargo run -q -p voice-agent-server --bin phase5-offline-preflight
