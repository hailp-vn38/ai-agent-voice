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
  scripts/agent-test.sh prune
  scripts/agent-test.sh web [test|typecheck|build]
  scripts/agent-test.sh run -- <command> [arguments...]

Examples:
  scripts/agent-test.sh targeted reminder
  scripts/agent-test.sh targeted reminder vela-server
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
default_crate="vela-server"

prune_stale_target_artifacts() {
  python3 "$script_dir/prune-target-artifacts.py" "$repo_root" "${1:-auto}"
}

build_targeted_args() {
  local filter="$1"
  local crate="$2"
  local crate_dir="$repo_root/crates/$crate"
  if [[ ! -d "$crate_dir" ]]; then
    command_args=(cargo test --quiet -p "$crate" "$filter")
    return
  fi
  if [[ -f "$crate_dir/tests/${filter}.rs" ]]; then
    command_args=(cargo test --quiet -p "$crate" --test "$filter")
    return
  fi
  local targets=()
  while IFS= read -r token; do
    [[ -n "$token" ]] && targets+=("$token")
  done < <(python3 - "$crate_dir" "$filter" <<'PY'
from pathlib import Path
import sys

crate_dir = Path(sys.argv[1])
flt = sys.argv[2]
src_dir = crate_dir / "src"
tests_dir = crate_dir / "tests"

in_src = "::" in flt
if not in_src and src_dir.is_dir():
    for path in src_dir.rglob("*.rs"):
        try:
            if flt in path.read_text(errors="ignore"):
                in_src = True
                break
        except OSError:
            pass

matched_tests = set()
fallback_all = False
if tests_dir.is_dir():
    for path in tests_dir.rglob("*.rs"):
        try:
            text = path.read_text(errors="ignore")
        except OSError:
            continue
        if flt not in text:
            continue
        rel = path.relative_to(tests_dir)
        top = rel.parts[0]
        if len(rel.parts) == 1:
            matched_tests.add(path.stem)
        elif (tests_dir / f"{top}.rs").is_file():
            matched_tests.add(top)
        else:
            fallback_all = True

# A test target with required-features cannot be named without them: cargo refuses the whole run.
manifest = crate_dir / "Cargo.toml"
if manifest.is_file():
    for block in manifest.read_text(errors="ignore").split("[[test]]")[1:]:
        block = block.split("\n[", 1)[0]
        if "required-features" in block:
            for line in block.splitlines():
                key, _, value = line.partition("=")
                if key.strip() == "name":
                    matched_tests.discard(value.strip().strip('"'))

if fallback_all and not matched_tests and not in_src:
    sys.exit(0)
if in_src or not matched_tests:
    print("--lib")
for name in sorted(matched_tests):
    print("--test")
    print(name)
PY
  )
  if (( ${#targets[@]} > 0 )); then
    command_args=(cargo test --quiet -p "$crate" "${targets[@]}" "$filter")
  else
    command_args=(cargo test --quiet -p "$crate" "$filter")
  fi
}

mode="${1:-help}"
if (( $# > 0 )); then shift; fi

case "$mode" in
  targeted)
    (( $# >= 1 && $# <= 2 )) || fail_usage 'targeted requires a test filter and optional crate'
    [[ -n "$1" ]] || fail_usage 'test filter must not be empty'
    build_targeted_args "$1" "${2:-$default_crate}"
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
  prune)
    (( $# == 0 )) || fail_usage 'prune takes no arguments'
    if ! prune_report="$(prune_stale_target_artifacts aggressive)"; then
      printf 'RESULT: FAIL (could not prune target artifacts)\n'
      exit 1
    fi
    printf 'RESULT: PASS (%s)\n' "$prune_report"
    exit 0
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

SECONDS=0
if (cd -- "$repo_root" && "${command_args[@]}") >"$log_file" 2>&1; then
  status=0
else
  status=$?
fi
elapsed="$SECONDS"
if [[ "${command_args[0]}" == "cargo" ]]; then
  prune_stale_target_artifacts auto >/dev/null 2>&1 || true
fi

if (( status == 0 )); then
  printf 'RESULT: PASS (%ss)\n' "$elapsed"
  grep -E 'test result: ok\.|Test Files[[:space:]]|^[[:space:]]*Tests[[:space:]]|^Ran [0-9]+ tests' "$log_file" \
    | tail -n 2 | cut -c "1-$max_cols" || true
  rm -f -- "$log_file"
  exit 0
fi

printf 'RESULT: FAIL (exit=%s, %ss)\nCOMMAND:' "$status" "$elapsed"
printf ' %q' "${command_args[@]}"
printf '\n'
python3 - "$log_file" "$max_lines" "$max_cols" <<'PY'
import re, sys
from pathlib import Path

lines = Path(sys.argv[1]).read_text(errors="replace").splitlines()
max_lines = int(sys.argv[2])
max_cols = int(sys.argv[3])

trigger = re.compile(
    r"(^|\s)error(\[[^\]]+\])?:|"
    r"^\s*-->\s+\S+:\d+:|"
    r"^----\s+.*\s+stdout\s+----|"
    r"panicked at\s+|"
    r"^test\s+.*\s+\.\.\.\s+FAILED|"
    r"^FAIL:\s+|"
    r'^\s*File ".*", line \d+|'
    r"^(AssertionError|ValueError|RuntimeError|KeyError|OSError|Error):|"
    r"^Caused by:|"
    r"^npm error|"
    r"^\s*[×✖]\s+"
)

selected = set()
for idx, line in enumerate(lines):
    if trigger.search(line):
        # Include surrounding lines so file:line locations and assertion values stay attached.
        start = max(0, idx - 1)
        end = min(len(lines), idx + 5)
        for k in range(start, end):
            selected.add(k)

if selected:
    print("ERRORS:")
    ordered = sorted(selected)
    if len(ordered) > max_lines:
        ordered = ordered[: max_lines // 2] + ordered[-(max_lines - max_lines // 2) :]
    prev = None
    for idx in ordered:
        if prev is not None and idx > prev + 1:
            print("  ...")
        print(f"{idx + 1}: {lines[idx][:max_cols]}")
        prev = idx
else:
    print(f"LOG TAIL (last {min(max_lines, len(lines))} lines):")
    for idx in range(max(0, len(lines) - max_lines), len(lines)):
        print(f"{idx + 1}: {lines[idx][:max_cols]}")
PY
printf 'FULL LOG: %s\n' "$log_file"
exit "$status"
