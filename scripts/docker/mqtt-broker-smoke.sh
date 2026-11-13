#!/usr/bin/env bash
# Smoke test of the Mosquitto broker config against a real broker (Docker required).
# Not part of the default test suite: ./scripts/docker/mqtt-broker-smoke.sh
#
# Starts the broker with docker/mosquitto/mosquitto.test.conf and a stub of the server's
# /mqtt/auth/* endpoints (scripts/docker/mqtt_auth_stub.py), then checks:
#   - anonymous and wrong-password clients are refused
#   - a Device subscribes to its own topic and is refused another Device's topic
#   - a Device cannot publish
#   - the service account publishes a retained Wake Signal and a reconnecting Device receives it
set -euo pipefail
cd "$(dirname "$0")/../.."

broker_image=${VELA_MOSQUITTO_IMAGE:-iegomez/mosquitto-go-auth:3.0.0-mosquitto_2.0.18}
stub_image=${SMOKE_STUB_IMAGE:-python:3.12-alpine}
suffix=$$
network=vela-mqtt-smoke-$suffix
broker=vela-mqtt-smoke-broker-$suffix
stub=vela-mqtt-smoke-stub-$suffix
secret=smoke-secret-$suffix
failures=0

cleanup() { docker rm -f "$broker" "$stub" >/dev/null 2>&1 || true; docker network rm "$network" >/dev/null 2>&1 || true; }
trap cleanup EXIT

mac() { python3 -c 'import hashlib,hmac,sys; print(hmac.new(sys.argv[1].encode(), sys.argv[2].encode(), hashlib.sha256).hexdigest())' "$secret" "$1"; }
topic() { python3 -c 'import hashlib,sys; print("vela/v1/devices/%s/signal" % hashlib.sha256(sys.argv[1].encode()).hexdigest()[:32])' "$1"; }
check() { # check <description> <expected 0|1 = condition result>
  if [[ $2 == 0 ]]; then echo "PASS  $1"; else echo "FAIL  $1"; failures=$((failures + 1)); fi
}
client() { docker exec "$broker" "$@"; }

service_password=$(python3 -c 'import hashlib,hmac,sys; k=hmac.new(sys.argv[1].encode(), b"vela-device-push/service-key", hashlib.sha256).digest(); print(hmac.new(k, b"v1:vela-server", hashlib.sha256).hexdigest())' "$secret")
password_a=$(mac v1:device-a)
topic_a=$(topic device-a)
topic_b=$(topic device-b)
payload='{"v":1,"type":"wake","id":"smoke","at":4102358400000,"expires_at":4102444800000}'

docker network create "$network" >/dev/null
docker run -d --name "$stub" --network "$network" --network-alias server -e SMOKE_SECRET="$secret" \
  -v "$PWD/scripts/docker/mqtt_auth_stub.py:/stub.py:ro" "$stub_image" python /stub.py >/dev/null
docker run -d --name "$broker" --network "$network" \
  -v "$PWD/docker/mosquitto/mosquitto.test.conf:/mosquitto/config/mosquitto.conf:ro" \
  -v "$PWD/docker/mosquitto/auth.d:/mosquitto/config/auth.d:ro" \
  "$broker_image" mosquitto -c /mosquitto/config/mosquitto.conf >/dev/null

ready=1
for _ in $(seq 1 30); do
  if client mosquitto_pub -u vela-server -P "$service_password" -t "$topic_b" -m ready -q 1 >/dev/null 2>&1; then ready=0; break; fi
  sleep 1
done
check "broker accepts the service account" "$ready"
[[ $ready == 0 ]] || { docker logs "$broker" 2>&1 | tail -20; exit 1; }

# Authentication
client mosquitto_pub -t "$topic_a" -m x -q 1 >/dev/null 2>&1 && rc=1 || rc=0
check "anonymous client is refused" "$rc"
client mosquitto_sub -u device-a -P wrong -t "$topic_a" -W 3 -C 1 >/dev/null 2>&1 && rc=1 || rc=0
check "wrong password is refused" "$rc"

# Retained Wake Signal reaches a Device, including after it reconnects
client mosquitto_pub -u vela-server -P "$service_password" -t "$topic_a" -m "$payload" -q 1 -r >/dev/null 2>&1 && rc=0 || rc=1
check "service account publishes a retained signal" "$rc"
for attempt in first reconnect; do
  received=$(client mosquitto_sub -u device-a -P "$password_a" -i vela-device-a -t "$topic_a" -q 1 -W 5 -C 1 2>/dev/null || true)
  [[ $received == "$payload" ]] && rc=0 || rc=1
  check "Device receives the retained signal ($attempt connection)" "$rc"
done

# Topic isolation: another Device's topic carries a retained message the Device must not see
client mosquitto_pub -u vela-server -P "$service_password" -t "$topic_b" -m "$payload" -q 1 -r >/dev/null 2>&1 || true
received=$(client mosquitto_sub -u device-a -P "$password_a" -t "$topic_b" -W 3 -C 1 2>/dev/null || true)
[[ -z $received ]] && rc=0 || rc=1
check "Device cannot read another Device's topic" "$rc"

# A Device cannot publish: listen on its own topic and look for a forged message
client sh -c "mosquitto_sub -u device-a -P '$password_a' -t '$topic_a' -W 4 > /tmp/forged.txt 2>/dev/null" &
listener=$!
sleep 1
client mosquitto_pub -u device-a -P "$password_a" -t "$topic_a" -m forged -q 1 >/dev/null 2>&1 || true
wait "$listener" || true
if client grep -q forged /tmp/forged.txt; then rc=1; else rc=0; fi
check "Device cannot publish" "$rc"

# Service account cannot subscribe
received=$(client mosquitto_sub -u vela-server -P "$service_password" -t "$topic_a" -W 3 -C 1 2>/dev/null || true)
[[ -z $received ]] && rc=0 || rc=1
check "service account cannot subscribe" "$rc"

if (( failures )); then echo "$failures check(s) failed"; docker logs "$broker" 2>&1 | tail -20; exit 1; fi
echo "mqtt broker smoke: all checks passed"
