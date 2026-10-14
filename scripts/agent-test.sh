#!/usr/bin/env bash
# Run project checks without flooding a coding agent's context.
# Full output is retained in a per-run log; terminal output is bounded.
set -euo pipefail

usage() {
  cat <<'HELP'
Usage:
  scripts/agent-test.sh targeted <test-filter> [crate]
  scripts/agent-test.sh package [crate]
  scripts/agent-test.sh full
  scripts/agent-test.sh check [crate]
  scripts/agent-test.sh web [test|typecheck|build]
  scripts/agent-test.sh run -- <command> [arguments...]

Examples:
  scripts/agent-test.sh targeted reminder
  scripts/agent-test.sh targeted reminder voice-agent-server
  scripts/agent-test.sh package
  scripts/agent-test.sh full
  scripts/agent-test.sh check
  scripts/agent-test.sh web test
  scripts/agent-test.sh run -- cargo clippy --workspace --all-targets -- -D warnings

Environment:
  AGENT_TEST_LOG_DIR    Full-log directory (default: ${TMPDIR:-/tmp}/ai-agent-voice-test-logs)
  AGENT_TEST_MAX_LINES  Maximum diagnostic lines on failure (default: 60)
  AGENT_TEST_MAX_COLS   Maximum characters per printed log line (default: 240)

The script always runs from the repository root and preserves the command exit code.
HELP
}

fail_usage() {
  printf 'ERROR: %s\n\n' "$1" >&2
  usage >&2
  exit 2
}

valid_positive_int() {
  [[ "$1" =~ ^[1-9][0-9]*$ ]] && (( ${#1} <= 6 ))
}

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "$script_dir/.." && pwd -P)"
web_root="$repo_root/apps/admin-web"
default_crate="voice-agent-server"

mode="${1:-help}"
if (( $# > 0 )); then shift; fi

case "$mode" in
  targeted)
    (( $# >= 1 && $# <= 2 )) || fail_usage 'targeted requires a test filter and optional crate'
    [[ -n "$1" ]] || fail_usage 'test filter must not be empty'
    command_args=(cargo test --quiet -p "${2:-$default_crate}" "$1")
    ;;
  package)
    (( $# <= 1 )) || fail_usage 'package accepts at most one crate'
    command_args=(cargo test --quiet -p "${1:-$default_crate}")
    ;;
  full)
    (( $# == 0 )) || fail_usage 'full takes no arguments'
    command_args=(cargo test --quiet --workspace)
    ;;
  check)
    (( $# <= 1 )) || fail_usage 'check accepts at most one crate'
    command_args=(cargo check --quiet -p "${1:-$default_crate}")
    ;;
  web)
    (( $# <= 1 )) || fail_usage 'web accepts at most one task'
    [[ -f "$web_root/package.json" ]] || fail_usage 'apps/admin-web/package.json not found'
    task="${1:-test}"
    case "$task" in
      test)      command_args=(npm --prefix "$web_root" run test -- --reporter=dot) ;;
      typecheck) command_args=(npm --prefix "$web_root" run typecheck) ;;
      build)     command_args=(npm --prefix "$web_root" run build) ;;
      *) fail_usage 'web task must be test, typecheck, or build' ;;
    esac
    ;;
  run)
    [[ "${1:-}" == '--' ]] || fail_usage 'run requires -- before the command'
    shift
    (( $# > 0 )) || fail_usage 'run requires a command'
    command_args=("$@")
    ;;
  -h|--help|help)
    usage
    exit 0
    ;;
  *) fail_usage "unknown mode: $mode" ;;
esac

max_lines="${AGENT_TEST_MAX_LINES:-60}"
max_cols="${AGENT_TEST_MAX_COLS:-240}"
valid_positive_int "$max_lines" || fail_usage 'AGENT_TEST_MAX_LINES must be a positive integer (at most 6 digits)'
valid_positive_int "$max_cols" || fail_usage 'AGENT_TEST_MAX_COLS must be a positive integer (at most 6 digits)'

if ! command -v "${command_args[0]}" >/dev/null 2>&1; then
  printf 'ERROR: required command not found: %s\n' "${command_args[0]}" >&2
  exit 127
fi

log_dir="${AGENT_TEST_LOG_DIR:-${TMPDIR:-/tmp}/ai-agent-voice-test-logs}"
mkdir -p -- "$log_dir"
log_dir="$(cd -- "$log_dir" && pwd -P)"
log_file="$(mktemp "$log_dir/agent-test.${mode}.XXXXXX")"

printf 'MODE: %s\nCOMMAND:' "$mode"
printf ' %q' "${command_args[@]}"
printf '\nLOG: %s\n' "$log_file"

SECONDS=0
if (cd -- "$repo_root" && "${command_args[@]}") >"$log_file" 2>&1; then
  status=0
else
  status=$?
fi
elapsed="$SECONDS"

if (( status == 0 )); then
  printf 'RESULT: PASS (%ss)\n' "$elapsed"
  # A few useful counters, if the test runner printed any.
  grep -E 'test result: ok\.|Test Files[[:space:]]|^[[:space:]]*Tests[[:space:]]' "$log_file" \
    | tail -n 3 | cut -c "1-$max_cols" || true
  exit 0
fi

printf 'RESULT: FAIL (exit=%s, %ss)\n' "$status" "$elapsed"
# Split the failure budget between indexed error lines and trailing context.
diag_lines=$(( max_lines / 2 ))
tail_lines=$(( max_lines - diag_lines ))
if (( diag_lines > 0 )); then
  printf '%s\n' 'DIAGNOSTICS (line numbers in full log):'
  grep -En '(^|[[:space:]])error(\[[^]]+\])?:|^test .* \.\.\. FAILED|^---- .* stdout ----|^failures:|^FAILED|^Error:|^npm error|^[[:space:]]*×|^[[:space:]]*✖|panicked at|^Caused by:' "$log_file" \
    | tail -n "$diag_lines" | cut -c "1-$max_cols" || true
fi
printf 'LOG TAIL (last %s lines):\n' "$tail_lines"
tail -n "$tail_lines" "$log_file" | cut -c "1-$max_cols"
printf 'FULL LOG: %s\n' "$log_file"
exit "$status"
