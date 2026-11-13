#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check-docker-storage.py
compose=(docker compose -p vela-test -f compose.test.yaml --profile checks)
"${compose[@]}" config --quiet
shutdown_log="$(mktemp)"
shutdown_pid=""
cleanup() {
  if [[ -n "$shutdown_pid" ]] && kill -0 "$shutdown_pid" 2>/dev/null; then
    kill "$shutdown_pid" 2>/dev/null || true
    wait "$shutdown_pid" 2>/dev/null || true
  fi
  rm -f "$shutdown_log"
  "${compose[@]}" down --volumes --remove-orphans
}
trap cleanup EXIT
"${compose[@]}" build
"${compose[@]}" run --rm --no-deps server-check
"${compose[@]}" run --rm --no-deps web-check
"${compose[@]}" up --detach --wait --wait-timeout 120 server web
"${compose[@]}" run --rm --no-deps smoke
"${compose[@]}" restart server
"${compose[@]}" up --detach --wait --wait-timeout 120 server web
"${compose[@]}" run --rm --no-deps smoke node /smoke.mjs after-restart

# Keep an SSE client open through Nginx while stopping the server container. The client emits a
# marker only after it has received both bounded-latency snapshots, so this also checks that the
# compose proxy keeps the stream open instead of buffering or cutting it off.
: >"$shutdown_log"
"${compose[@]}" run --rm --no-deps smoke node /smoke.mjs system-live-shutdown >"$shutdown_log" 2>&1 &
shutdown_pid=$!
ready=0
for ((attempt = 0; attempt < 40; attempt += 1)); do
  if rg -q '^SYSTEM_LIVE_WAITING_FOR_SERVER_STOP$' "$shutdown_log"; then
    ready=1
    break
  fi
  if ! kill -0 "$shutdown_pid" 2>/dev/null; then break; fi
  sleep 0.5
done
if [[ "$ready" != 1 ]]; then
  tail -n 60 "$shutdown_log" >&2
  wait "$shutdown_pid" || true
  shutdown_pid=""
  exit 1
fi
"${compose[@]}" stop server
if ! wait "$shutdown_pid"; then
  tail -n 60 "$shutdown_log" >&2
  shutdown_pid=""
  exit 1
fi
shutdown_pid=""
rg -q '^SYSTEM_LIVE_SERVER_STOP_CLOSED$' "$shutdown_log"
printf '%s\n' 'System Live shutdown through Nginx passed'
