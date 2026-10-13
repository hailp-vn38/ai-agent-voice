#!/usr/bin/env bash
# Build the repo-owned proxy with the same pinned Python base as Plugin workers.
# Run against the dedicated rootless Linux Engine; record the printed image ID.
set -euo pipefail

if (( $# != 1 )) || [[ "$1" != python:3.12-slim@sha256:* ]]; then
  printf 'Usage: %s python:3.12-slim@sha256:<64-hex-digest>\n' "$0" >&2
  exit 2
fi
base_image="$1"
base_digest="${base_image##*sha256:}"
if [[ ! "$base_digest" =~ ^[[:xdigit:]]{64}$ ]]; then
  printf 'Invalid pinned Python image digest\n' >&2
  exit 2
fi
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source_dir="$script_dir/../deploy/agent-egress-proxy"
source_sha="$(sha256sum "$source_dir/proxy.py" | cut -d ' ' -f 1)"
tag="agent-egress-proxy:source-${source_sha:0:16}"
docker build --network=none --pull=false --no-cache \
  --build-arg "BASE_IMAGE=$base_image" \
  --build-arg "SOURCE_SHA256=$source_sha" \
  --tag "$tag" "$source_dir"
docker image inspect --format '{{.Id}}' "$tag"
