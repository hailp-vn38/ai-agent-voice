#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
Usage: ./scripts/compose.sh <prod|test> <command> [server|web]
       ./scripts/compose.sh [prod|test] build [server web|all]
       ./scripts/compose.sh <prod|test> down [server web|all]

  build     Build images (checks /mnt/storage first)
  up        Start the stack and wait for readiness (checks storage first)
  down      Remove containers/networks; keep data
  restart   Restart existing containers
  logs      Follow the last 100 log lines
  status    Show containers, including stopped ones
  config    Validate Compose without printing secrets
  check     Check application storage
  run       Run all tests and smoke checks (test only; removes test data)

Build defaults to prod; omit services or use all to build all images.
Build and down accept multiple services; down without services or with all removes the whole stack.
Optional server/web selection: up, restart, logs, status.
Production setup: docs/docker.md
EOF
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then
  usage
  exit 0
fi
if [[ ${1:-} == build ]]; then
  set -- prod "$@"
fi
if (( $# < 2 )); then
  usage >&2
  exit 2
fi

environment=$1
action=$2
if [[ $action != build && $action != down ]] && (( $# > 3 )); then
  usage >&2
  exit 2
fi
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
  build|down)
    if [[ $# == 3 && $3 == all ]]; then
      selection=()
    else
      for service in "${@:3}"; do
        case "$service" in
          server|web) selection+=("$service") ;;
          *) usage >&2; exit 2 ;;
        esac
      done
    fi
    ;;
  up|restart|logs|status)
    if (( $# == 3 )); then
      case "$3" in
        server|web) selection=("$3") ;;
        *) usage >&2; exit 2 ;;
      esac
    fi
    ;;
  config|check)
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
  build) exec "${compose[@]}" build ${selection[@]+"${selection[@]}"} ;;
  up) exec "${compose[@]}" up --detach --wait --wait-timeout "$wait_seconds" ${selection[@]+"${selection[@]}"} ;;
  down) exec "${compose[@]}" down ${selection[@]+"${selection[@]}"} ;;
  restart) exec "${compose[@]}" restart ${selection[@]+"${selection[@]}"} ;;
  logs) exec "${compose[@]}" logs --follow --tail 100 ${selection[@]+"${selection[@]}"} ;;
  status) exec "${compose[@]}" ps --all ${selection[@]+"${selection[@]}"} ;;
  config) exec "${compose[@]}" config --quiet ;;
  check) exit 0 ;;
esac
