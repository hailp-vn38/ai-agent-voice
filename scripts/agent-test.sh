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

prune_stale_target_artifacts() {
  local mode_arg="${1:-auto}"
  python3 - "$repo_root" "$mode_arg" <<'PY' 2>/dev/null || true
import os, re, shutil, sys
from pathlib import Path

root = Path(sys.argv[1])
mode = sys.argv[2]
target_dir = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")) / "debug"
if not target_dir.is_dir():
    sys.exit(0)

keep = 1 if mode == "aggressive" else 2
inc_dir = target_dir / "incremental"
if inc_dir.is_dir():
    groups = {}
    for entry in inc_dir.iterdir():
        if entry.is_dir() and "-" in entry.name:
            prefix = entry.name.rsplit("-", 1)[0]
            try:
                mtime = entry.stat().st_mtime
            except OSError:
                continue
            groups.setdefault(prefix, []).append((mtime, entry))
    for entries in groups.values():
        entries.sort(key=lambda item: item[0], reverse=True)
        for _, old_dir in entries[keep:]:
            shutil.rmtree(old_dir, ignore_errors=True)

deps_dir = target_dir / "deps"
if deps_dir.is_dir():
    test_dir = root / "crates" / "voice-agent-server" / "tests"
    test_stems = {p.stem.replace("-", "_") for p in test_dir.glob("*.rs")} if test_dir.is_dir() else set()
    tracked_prefixes = test_stems | {"voice_agent_server", "libvoice_agent_server", "plugin_runtime", "provider_bench", "provider_bench_av", "provider_runtime_bench", "zerotts_core_check"}
    pattern = re.compile(r"^([a-zA-Z0-9_]+)-([0-9a-f]{16})(\..+)?$")
    groups = {}
    for entry in deps_dir.iterdir():
        m = pattern.match(entry.name)
        if not m:
            continue
        prefix, digest, ext = m.group(1), m.group(2), m.group(3) or ""
        if ext in (".d", ".dSYM"):
            continue
        if prefix not in tracked_prefixes:
            continue
        try:
            mtime = entry.stat().st_mtime
        except OSError:
            continue
        groups.setdefault((prefix, ext), []).append((mtime, prefix, digest, entry))
    for (prefix, ext), entries in groups.items():
        entries.sort(key=lambda item: item[0], reverse=True)
        limit = 0 if (mode == "aggressive" and prefix in test_stems and ext == "") else keep
        for _, pfx, digest, old_path in entries[limit:]:
            for candidate in (old_path, deps_dir / f"{pfx}-{digest}.d", deps_dir / f"{pfx}-{digest}.dSYM"):
                if candidate.is_dir():
                    shutil.rmtree(candidate, ignore_errors=True)
                elif candidate.exists():
                    try:
                        candidate.unlink()
                    except OSError:
                        pass
PY
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
    prune_stale_target_artifacts aggressive
    printf 'RESULT: PASS (pruned stale target artifacts)\n'
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
  prune_stale_target_artifacts auto
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
