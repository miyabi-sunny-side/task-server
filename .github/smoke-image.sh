#!/usr/bin/env bash
# Exercise the image with its default empty ledger and no backup configuration.
set -euo pipefail
image=${1:?usage: smoke-image.sh IMAGE [PORT]}
listen_port=${2:-3000}
smoke_test=${TASK_SERVER_SMOKE_TEST:-}
if [[ -z "$smoke_test" ]]; then
  smoke_test=$(cargo test --locked --test ui_smoke --no-run --message-format=json | jq -er 'select(.reason == "compiler-artifact" and .target.name == "ui_smoke" and .profile.test) | .executable')
fi
test -x "$smoke_test"
container=$(docker run --detach --env PORT="$listen_port" --env APP_BIND_ADDR=invalid-legacy-address --publish "127.0.0.1::$listen_port" "$image")
trap 'docker rm --force --volumes "$container" >/dev/null' EXIT
port=$(docker port "$container" "$listen_port/tcp")
base="http://$port"

for attempt in {1..40}; do
  if curl --fail --silent "$base/healthz" >/dev/null; then
    break
  fi
  if [[ "$attempt" == 40 ]]; then
    docker logs "$container"
    exit 1
  fi
  sleep 0.5
done

test "$(curl --fail --silent "$base/healthz")" = "ok"
test "$(curl --fail --silent "$base/api/health")" = '{"status":"ok"}'
TASK_SERVER_SMOKE_URL="$base" "$smoke_test" --exact packaged_ui --nocapture
docker exec "$container" sh -c 'test "$(id -u)" != 0 && test -d /app/data/ledger/tasks && test -w /app/data/ledger/tasks && test ! -e /app/client'
