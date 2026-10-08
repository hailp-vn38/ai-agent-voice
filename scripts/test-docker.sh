#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/check-docker-storage.py
compose=(docker compose -p voice-agent-test -f compose.test.yaml --profile checks)
"${compose[@]}" config --quiet
trap '"${compose[@]}" down --volumes --remove-orphans' EXIT
"${compose[@]}" build
"${compose[@]}" run --rm --no-deps server-check
"${compose[@]}" run --rm --no-deps web-check
"${compose[@]}" up --detach --wait --wait-timeout 120 server web
"${compose[@]}" run --rm --no-deps smoke
"${compose[@]}" restart server
"${compose[@]}" up --detach --wait --wait-timeout 120 server web
"${compose[@]}" run --rm --no-deps smoke node /smoke.mjs after-restart
