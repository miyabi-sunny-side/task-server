#!/usr/bin/env bash
#
# Restore drill: prove that the backup contract actually restores.
#
# A backup nobody has restored is a hope, not a backup. This script runs the
# whole loop on one machine, with public images and no real credentials:
#
#   1. start a local S3-compatible object store (MinIO) standing in for R2
#   2. start task-server and a Litestream sidecar on one shared local volume,
#      using deploy/litestream.yml.example unchanged
#   3. write a product and a task through the HTTP API while both are running
#   4. stop both, and restore the replica into a brand new empty volume
#   5. start task-server on the restored volume and read the same task back
#   6. remove every container, volume, and network it created
#
# The MinIO credentials are generated here and live only for this run. No R2
# endpoint, bucket, or key is used or needed.
#
# Usage: deploy/restore-drill.sh
#
#   TASK_SERVER_IMAGE   image to drill (default: built from this repository)
#   DRILL_PORT          host port for the live server (default: 39311)
#   DRILL_RESTORED_PORT host port for the restored server (default: 39312)

set -euo pipefail

readonly LITESTREAM_IMAGE="litestream/litestream:0.5.16"
readonly MINIO_IMAGE="minio/minio:RELEASE.2025-07-23T15-54-02Z"
readonly PREFIX="task-server-drill"
readonly NETWORK="${PREFIX}-net"
readonly LIVE_VOLUME="${PREFIX}-live"
readonly RESTORED_VOLUME="${PREFIX}-restored"
readonly MINIO_VOLUME="${PREFIX}-minio"
readonly DB_PATH="/app/data/task-server.db"
readonly BUCKET="drill-backups"
readonly PORT="${DRILL_PORT:-39311}"
readonly RESTORED_PORT="${DRILL_RESTORED_PORT:-39312}"
readonly IDENTITY="drill"
readonly CSRF="drill-csrf"
readonly TASK_ID="drill-survivor"
readonly TASK_TITLE="written before the restore"
readonly PRODUCT_ID="drill/queue"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly repo_root
readonly CONFIG="${repo_root}/deploy/litestream.yml.example"

step() { printf '\n=== %s\n' "$*"; }

random_secret() { head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n'; }

cleanup() {
  step "cleaning up"
  docker rm --force "${PREFIX}-app" "${PREFIX}-restored-app" \
    "${PREFIX}-litestream" "${PREFIX}-minio" >/dev/null 2>&1 || true
  docker volume rm --force "${LIVE_VOLUME}" "${RESTORED_VOLUME}" \
    "${MINIO_VOLUME}" >/dev/null 2>&1 || true
  docker network rm "${NETWORK}" >/dev/null 2>&1 || true
  echo "removed the drill containers, volumes, and network"
}

# Wait for an HTTP endpoint to answer, so the drill never races a slow start.
wait_for_http() {
  local url="$1" name="$2" attempt=0
  while [[ "${attempt}" -lt 60 ]]; do
    if curl --fail --silent --show-error --max-time 2 "${url}" >/dev/null 2>&1; then
      return 0
    fi
    attempt=$((attempt + 1))
    sleep 0.5
  done
  echo "${name} never answered ${url}" >&2
  docker logs "${name}" >&2 || true
  return 1
}

api() {
  local method="$1" port="$2" path="$3"
  shift 3
  curl --fail --silent --show-error \
    --request "${method}" \
    --header "X-Auth-User: ${IDENTITY}" \
    --header "X-CSRF-Token: ${CSRF}" \
    --header "Origin: http://127.0.0.1:${port}" \
    --header "Content-Type: application/json" \
    "$@" \
    "http://127.0.0.1:${port}${path}"
}

command -v docker >/dev/null || { echo "docker is required" >&2; exit 1; }
command -v curl >/dev/null || { echo "curl is required" >&2; exit 1; }

image="${TASK_SERVER_IMAGE:-}"
if [[ -z "${image}" ]]; then
  step "building the task-server image"
  image="task-server:drill"
  docker build --tag "${image}" "${repo_root}"
fi
readonly image

# Leftovers from an interrupted run would poison the drill.
cleanup
trap cleanup EXIT

access_key="drill$(random_secret)"
secret_key="$(random_secret)"

step "starting MinIO as the R2 stand-in"
docker network create "${NETWORK}" >/dev/null
docker run --detach --name "${PREFIX}-minio" --network "${NETWORK}" \
  --env "MINIO_ROOT_USER=${access_key}" \
  --env "MINIO_ROOT_PASSWORD=${secret_key}" \
  --volume "${MINIO_VOLUME}:/data" \
  "${MINIO_IMAGE}" server /data >/dev/null
docker run --rm --network "${NETWORK}" --entrypoint sh "${MINIO_IMAGE}" -c "
  set -e
  for attempt in \$(seq 1 60); do
    mc alias set drill http://${PREFIX}-minio:9000 '${access_key}' '${secret_key}' >/dev/null 2>&1 && break
    sleep 0.5
  done
  mc mb --ignore-existing drill/${BUCKET}
"

step "starting task-server and the Litestream sidecar on one shared volume"
docker run --detach --name "${PREFIX}-app" --network "${NETWORK}" \
  --publish "127.0.0.1:${PORT}:3000" \
  --volume "${LIVE_VOLUME}:/app/data" \
  --env "APP_CSRF_TOKEN=${CSRF}" \
  "${image}" >/dev/null
wait_for_http "http://127.0.0.1:${PORT}/healthz" "${PREFIX}-app"
echo "task-server is up on 127.0.0.1:${PORT}"

# The sidecar runs the example config unchanged and reads the bucket out of the
# environment, exactly as a deployment against R2 would. It shares the app's
# uid so it can write beside the database on the shared volume.
docker run --detach --name "${PREFIX}-litestream" --network "${NETWORK}" \
  --user 10001:10001 \
  --volume "${LIVE_VOLUME}:/app/data" \
  --volume "${CONFIG}:/etc/litestream.yml:ro" \
  --env "R2_ENDPOINT=http://${PREFIX}-minio:9000" \
  --env "R2_BUCKET=${BUCKET}" \
  --env "R2_PREFIX=task-server" \
  --env "R2_ACCESS_KEY_ID=${access_key}" \
  --env "R2_SECRET_ACCESS_KEY=${secret_key}" \
  "${LITESTREAM_IMAGE}" replicate >/dev/null
sleep 3
docker logs --tail 8 "${PREFIX}-litestream"

step "writing a product and a task through the HTTP API while replication runs"
api PUT "${PORT}" "/api/products/${PRODUCT_ID}" \
  --data '{"repository":"https://example.test/drill/queue.git","description":"restore drill"}'
echo
api POST "${PORT}" "/api/tasks" \
  --data "{\"id\":\"${TASK_ID}\",\"title\":\"${TASK_TITLE}\",\"product_id\":\"${PRODUCT_ID}\"}" \
  --output /dev/null --write-out 'created task, HTTP %{http_code}\n'

# Give the sidecar time to ship the frames the writes produced.
sleep 5

step "stopping the server and the sidecar"
docker stop "${PREFIX}-app" >/dev/null
sleep 3
docker stop "${PREFIX}-litestream" >/dev/null
docker logs --tail 20 "${PREFIX}-litestream"

step "restoring into a brand new empty volume"
# A fresh volume mounted over the image's /app/data inherits that directory's
# ownership, which is the unprivileged uid both the server and the sidecar use.
docker run --rm --entrypoint true --volume "${RESTORED_VOLUME}:/app/data" "${image}"
docker run --rm --network "${NETWORK}" --user 10001:10001 \
  --volume "${RESTORED_VOLUME}:/app/data" \
  --volume "${CONFIG}:/etc/litestream.yml:ro" \
  --env "R2_ENDPOINT=http://${PREFIX}-minio:9000" \
  --env "R2_BUCKET=${BUCKET}" \
  --env "R2_PREFIX=task-server" \
  --env "R2_ACCESS_KEY_ID=${access_key}" \
  --env "R2_SECRET_ACCESS_KEY=${secret_key}" \
  "${LITESTREAM_IMAGE}" restore -integrity-check full "${DB_PATH}"

step "starting task-server on the restored volume"
docker run --detach --name "${PREFIX}-restored-app" --network "${NETWORK}" \
  --publish "127.0.0.1:${RESTORED_PORT}:3000" \
  --volume "${RESTORED_VOLUME}:/app/data" \
  --env "APP_CSRF_TOKEN=${CSRF}" \
  "${image}" >/dev/null
wait_for_http "http://127.0.0.1:${RESTORED_PORT}/healthz" "${PREFIX}-restored-app"

step "reading the task back out of the restored database"
restored_task="$(api GET "${RESTORED_PORT}" "/api/tasks/${TASK_ID}")"
echo "${restored_task}"
restored_product="$(api GET "${RESTORED_PORT}" "/api/products/${PRODUCT_ID}")"
echo "${restored_product}"

case "${restored_task}" in
  *"\"${TASK_TITLE}\""*) ;;
  *) echo "the restored database does not carry the task written before the restore" >&2; exit 1 ;;
esac
case "${restored_product}" in
  *"\"${PRODUCT_ID}\""*) ;;
  *) echo "the restored database does not carry the product written before the restore" >&2; exit 1 ;;
esac

step "DRILL PASSED: the task written during replication survived the restore"
