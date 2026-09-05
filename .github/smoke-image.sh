#!/usr/bin/env bash
# Exercise the image with its default empty ledger and no backup configuration.
set -euo pipefail
image=${1:?usage: smoke-image.sh IMAGE}
container=$(docker run --detach --publish 127.0.0.1::3000 "$image")
trap 'docker rm --force --volumes "$container" >/dev/null' EXIT
port=$(docker port "$container" 3000/tcp)
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
curl --fail --silent "$base/" | grep --ignore-case '<!doctype html'
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "$base/api/missing")" = "404"
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "$base/projects/example")" = "200"
docker exec "$container" sh -c 'test "$(id -u)" != 0 && test -d /app/data/ledger/tasks && test -w /app/data/ledger/tasks'
curl --fail --silent "$base/worker/snapshot" | python3 -c 'import json,sys; assert "tasks" in json.load(sys.stdin)'
