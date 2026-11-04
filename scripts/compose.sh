#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
Usage: ./scripts/compose.sh <prod|test> <command> [server|web]
       ./scripts/compose.sh [prod|test] build [server web|all]
       ./scripts/compose.sh <prod|test> down [server web|all]

  build     Build images (checks /mnt/storage first)
  pull      Pull images from the registry set by VELA_SERVER_IMAGE/VELA_WEB_IMAGE
  up        Start the stack and wait for readiness (checks storage first)
  down      Remove containers/networks; keep data
  restart   Restart existing containers
  tools-update  Install the latest yt-dlp into data/bin (prod; server must be running)
  logs      Follow the last 100 log lines
  status    Show containers, including stopped ones
  config    Validate Compose without printing secrets
  check     Check application storage
  run       Run all tests and smoke checks (test only; removes test data)

Build defaults to prod; omit services or use all to build all images.
Build and down accept multiple services; down without services or with all removes the whole stack.
Optional server/web selection: up, pull, restart, logs, status.
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
    compose=(docker compose -p vela-prod -f compose.yaml -f compose.prod.yaml)
    wait_seconds=1800
    ;;
  test)
    compose=(docker compose -p vela-test -f compose.test.yaml)
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
  up|pull|restart|logs|status)
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
  tools-update)
    if [[ $environment != prod ]] || (( $# == 3 )); then usage >&2; exit 2; fi
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

# Rootless Docker maps container root to the invoking host user, so bind-mounted files the
# host user owns are only readable as container root. Rootful Docker runs as the host user.
if [[ $environment == prod && ( $action == up || $action == restart ) && -z ${APP_UID:-} ]]; then
  if docker info --format '{{.SecurityOptions}}' 2>/dev/null | grep -q rootless; then
    APP_UID=0 APP_GID=0
  else
    APP_UID=$(id -u) APP_GID=$(id -g)
  fi
fi
export APP_UID=${APP_UID:-} APP_GID=${APP_GID:-}

case "$action" in
  build) exec "${compose[@]}" build ${selection[@]+"${selection[@]}"} ;;
  pull) exec "${compose[@]}" pull ${selection[@]+"${selection[@]}"} ;;
  up) exec "${compose[@]}" up --detach --wait --wait-timeout "$wait_seconds" ${selection[@]+"${selection[@]}"} ;;
  down) exec "${compose[@]}" down ${selection[@]+"${selection[@]}"} ;;
  restart) exec "${compose[@]}" restart ${selection[@]+"${selection[@]}"} ;;
  logs) exec "${compose[@]}" logs --follow --tail 100 ${selection[@]+"${selection[@]}"} ;;
  status) exec "${compose[@]}" ps --all ${selection[@]+"${selection[@]}"} ;;
  config) exec "${compose[@]}" config --quiet ;;
  check) exit 0 ;;
  tools-update)
    # Overrides the image's pinned yt-dlp via /app/data/bin, which is first on PATH.
    exec "${compose[@]}" exec -T server sh -euc '
      case "$(uname -m)" in x86_64) asset=yt-dlp_linux ;; aarch64) asset=yt-dlp_linux_aarch64 ;; *) exit 1 ;; esac
      base=https://github.com/yt-dlp/yt-dlp/releases/latest/download
      mkdir -p /app/data/bin && cd /app/data/bin
      curl --fail --silent --show-error --location --retry 3 --output yt-dlp.new "$base/$asset"
      curl --fail --silent --show-error --location --retry 3 "$base/SHA2-256SUMS" \
        | awk -v a="$asset" "\$2 == a { print \$1 \"  yt-dlp.new\" }" | sha256sum -c -
      chmod 755 yt-dlp.new
      ./yt-dlp.new --version
      mv yt-dlp.new yt-dlp
      echo "yt-dlp on PATH: $(command -v yt-dlp) $(yt-dlp --version)"' ;;
esac
