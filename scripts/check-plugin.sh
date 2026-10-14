#!/usr/bin/env bash
# Validate Python or Rust packages through the shared Plugin Store contracts.
set -euo pipefail
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
if [[ "${1:-}" == '--static-only' && $# == 2 ]]; then
  args=(--static-only "$(realpath -- "$2")")
elif (( $# == 2 || $# == 3 )); then
  args=("$(realpath -- "$1")" "$(realpath -- "$2")" "$(realpath -- "${3:-$repo_root/config.toml}")")
else
  echo 'Usage: scripts/check-plugin.sh PACKAGE.zip CASES.json [CONFIG.toml]' >&2
  echo '       scripts/check-plugin.sh --static-only PACKAGE.zip' >&2
  exit 2
fi
exec "$repo_root/scripts/agent-test.sh" run -- cargo run --quiet --locked -p voice-agent-server --example check-plugin -- "${args[@]}"
