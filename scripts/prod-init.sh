#!/usr/bin/env bash
# Prepare a fresh checkout for `./scripts/compose.sh prod up`.
# Creates only missing files, so it is safe to run again; existing secrets are never replaced.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
Usage: ./scripts/prod-init.sh [--registry NAMESPACE] [--tag TAG]

  --registry NAMESPACE  Use pulled images NAMESPACE/vela-server and NAMESPACE/vela-web
                        (written to .env). Omit to build images locally.
  --tag TAG             Image tag for --registry (default: latest)

Creates when missing: docker/config.prod.toml (random tokens), docker/secrets.env,
docker/credential-keys.json (random AES-256 key), runtime/kokoro-vi, and the
data/models directories from VOICE_PROD_DATA_DIR / VOICE_PROD_MODELS_DIR.
EOF
}

registry="" tag=latest
while (( $# )); do
  case "$1" in
    --registry) registry=${2:?}; shift 2 ;;
    --tag) tag=${2:?}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

random_token() { python3 -c 'import secrets; print(secrets.token_urlsafe(32))'; }
created() { printf 'created  %s\n' "$1"; }
kept() { printf 'kept     %s\n' "$1"; }

umask 077

config=docker/config.prod.toml
if [[ -e $config ]]; then
  kept "$config"
else
  VOICE_TOKEN=$(random_token) JWT_SECRET=$(random_token) python3 - "$config" <<'EOF'
import os, sys
text = open("docker/config.prod.example.toml").read()
text = text.replace("REPLACE_WITH_VOICE_TOKEN", os.environ["VOICE_TOKEN"])
text = text.replace("REPLACE_WITH_RANDOM_JWT_SECRET_OF_32_CHARS_OR_MORE", os.environ["JWT_SECRET"])
open(sys.argv[1], "w").write(text)
EOF
  created "$config (edit provider keys and public_ws_url)"
fi

if [[ -e docker/secrets.env ]]; then
  kept docker/secrets.env
else
  cp docker/secrets.env.example docker/secrets.env
  created docker/secrets.env
fi

keys=docker/credential-keys.json
if [[ -e $keys ]]; then
  kept "$keys"
else
  python3 - "$keys" <<'EOF'
import base64, json, secrets, sys
key = base64.b64encode(secrets.token_bytes(32)).decode()
json.dump({"current_version": 1, "keys": {"1": key}}, open(sys.argv[1], "w"))
EOF
  created "$keys (back it up: stored credentials cannot be decrypted without it)"
fi

# Rootless Docker reads bind mounts as the host user; keep them readable to that user only.
chmod 600 "$config" docker/secrets.env "$keys"

umask 022
# Kokoro is optional; the mount only needs the directory to exist.
if [[ ! -d runtime/kokoro-vi ]]; then
  mkdir -p runtime/kokoro-vi
  created runtime/kokoro-vi
fi

for dir in "${VOICE_PROD_DATA_DIR:-/mnt/storage/ai-agent-voice/docker/prod/data}" \
           "${VOICE_PROD_MODELS_DIR:-/mnt/storage/ai-agent-voice/docker/prod/models}"; do
  if [[ -d $dir ]]; then
    kept "$dir"
  elif mkdir -p "$dir" 2>/dev/null; then
    created "$dir"
  else
    printf 'warning  cannot create %s; create it (writable by you) or set VOICE_PROD_*_DIR in .env\n' "$dir" >&2
  fi
done

if [[ -n $registry ]]; then
  touch .env
  for pair in "VELA_SERVER_IMAGE=$registry/vela-server:$tag" "VELA_WEB_IMAGE=$registry/vela-web:$tag"; do
    name=${pair%%=*}
    if grep -q "^$name=" .env; then
      sed -i.bak "s|^$name=.*|$pair|" .env && rm -f .env.bak
    else
      printf '%s\n' "$pair" >> .env
    fi
  done
  created ".env images from $registry (tag $tag)"
fi

cat <<EOF

Next:
  1. Edit $config: provider API keys, server.public_ws_url (address devices use).
EOF
if [[ -n $registry ]]; then
  echo "  2. docker login (private images), then ./scripts/compose.sh prod pull"
else
  echo "  2. ./scripts/compose.sh prod build"
fi
echo "  3. ./scripts/compose.sh prod up"
