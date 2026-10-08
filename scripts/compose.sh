#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
Usage: bash scripts/compose.sh <prod|test> <command> [server|web]

  build     Build images (checks /mnt/storage first)
  up        Start the stack and wait for readiness (checks storage first)
  down      Remove containers/networks; keep data
  restart   Restart existing containers
  logs      Follow the last 100 log lines
  status    Show containers, including stopped ones
  config    Validate Compose without printing secrets
  check     Check Docker storage
  run       Run all tests and smoke checks (test only; removes test data)

Optional server/web selection: build, up, restart, logs, status.
Production setup: docs/docker.md
EOF
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then
  usage
  exit 0
fi
if (( $# < 2 || $# > 3 )); then
  usage >&2
  exit 2
fi

environment=$1
action=$2
selection=()
case "$environment" in
  prod)
    compose=(docker compose -p voice-agent-prod -f compose.yaml -f compose.prod.yaml)
    wait_seconds=1800
    ;;
  test)
    compose=(docker compose -p voice-agent-test -f compose.test.yaml)
    wait_seconds=120
    ;;
  *) usage >&2; exit 2 ;;
esac

case "$action" in
  build|up|restart|logs|status)
    if (( $# == 3 )); then
      case "$3" in
        server|web) selection=("$3") ;;
        *) usage >&2; exit 2 ;;
      esac
    fi
    ;;
  down|config|check)
    if (( $# == 3 )); then usage >&2; exit 2; fi
    ;;
  run)
    if [[ $environment != test ]] || (( $# == 3 )); then usage >&2; exit 2; fi
    exec bash scripts/test-docker.sh
    ;;
  *) usage >&2; exit 2 ;;
esac

case "$action" in
  build|up|check) python3 scripts/check-docker-storage.py ;;
esac

case "$action" in
  build) exec "${compose[@]}" build "${selection[@]}" ;;
  up) exec "${compose[@]}" up --detach --wait --wait-timeout "$wait_seconds" "${selection[@]}" ;;
  down) exec "${compose[@]}" down ;;
  restart) exec "${compose[@]}" restart "${selection[@]}" ;;
  logs) exec "${compose[@]}" logs --follow --tail 100 "${selection[@]}" ;;
  status) exec "${compose[@]}" ps --all "${selection[@]}" ;;
  config) exec "${compose[@]}" config --quiet ;;
  check) exit 0 ;;
esac
