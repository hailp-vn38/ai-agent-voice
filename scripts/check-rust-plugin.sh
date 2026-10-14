#!/usr/bin/env bash
# Compatibility entry point; Python and Rust use the same package checker.
set -euo pipefail
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
exec "$repo_root/scripts/check-plugin.sh" "$@"
