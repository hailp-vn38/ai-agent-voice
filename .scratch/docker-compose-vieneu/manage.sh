#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export DOCKER_HOST=unix:///mnt/storage/ai-agent-voice/docker/engine/run/docker.sock
exec docker compose --env-file docker/secrets.env -p voice-agent-prod \
  -f compose.yaml -f compose.prod.yaml -f compose.vieneu.yaml \
  -f .scratch/docker-compose-vieneu/compose.runtime.yaml "$@"
