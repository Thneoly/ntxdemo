#!/usr/bin/env bash
set -euo pipefail

log() {
  echo "[ntx-backend-container] $*"
}

die() {
  echo "error: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

need_cmd ntx-backend
need_cmd oras
need_cmd bash

# Defaults (overridable via env)
BIND="${NTX_BACKEND_BIND:-0.0.0.0:9090}"
DATA_DIR="${NTX_BACKEND_DATA_DIR:-/data}"
CORS_ANY_ORIGIN="${NTX_BACKEND_CORS_ANY_ORIGIN:-true}"
ORAS_BIN="${NTX_BACKEND_ORAS_BIN:-oras}"

CONFIG_PATH="${NTX_BACKEND_CONFIG:-/app/config/ntx-backend.yaml}"
WAC_CWD="${NTX_BACKEND_WAC_COMPOSE_CWD:-/app}"
WAC_BIN="${NTX_BACKEND_WAC_COMPOSE_BIN:-/usr/local/bin/ntx-wac-compose}"
NTX_BIN="${NTX_BACKEND_NTX_BIN:-/usr/local/bin/ntx}"
WASM_ARTIFACT_TYPE="${WASM_ARTIFACT_TYPE:-application/vnd.ntx.action-executor.v1}"

mkdir -p "$(dirname "$CONFIG_PATH")" "$DATA_DIR" /app/component/wac/deps/component

# Generate a minimal config file each start.
# (Keeps container behavior consistent and avoids leaking host paths.)
{
  echo "bind: \"$BIND\""
  echo "data_dir: \"$DATA_DIR\""
  echo "cors_any_origin: ${CORS_ANY_ORIGIN}"
  echo "oras_bin: \"$ORAS_BIN\""
  echo "ntx_bin: \"$NTX_BIN\""
  echo "wac_compose_bin: \"$WAC_BIN\""
  echo "wac_compose_cwd: \"$WAC_CWD\""
  echo "wasm_artifact_type: \"$WASM_ARTIFACT_TYPE\""

  if [[ -n "${HARBOR_CA_FILE:-}" || -n "${HARBOR_USER:-}" || -n "${HARBOR_PASS:-}" ]]; then
    echo "harbor:"
    [[ -n "${HARBOR_CA_FILE:-}" ]] && echo "  ca_file: ${HARBOR_CA_FILE}"
    [[ -n "${HARBOR_USER:-}" ]] && echo "  user: ${HARBOR_USER}"
    [[ -n "${HARBOR_PASS:-}" ]] && echo "  pass: ${HARBOR_PASS}"
  fi
} >"$CONFIG_PATH"

# Optional: pull eventbus/scheduler wasm deps into /app/component/wac/deps/component
# so /api/v1/wasm/push can compose scheduler-composed.wasm.
if [[ "${PULL_WAC_DEPS_ON_START:-0}" == "1" ]]; then
  registry="${HARBOR_REGISTRY:-192.168.31.138}"
  tag="${WASM_TAG:-v0.0.1}"
  eventbus_ref="${HARBOR_EVENTBUS_REF:-$registry/ntx/eventbus:$tag}"
  scheduler_ref="${HARBOR_SCHEDULER_REF:-$registry/ntx/scheduler:$tag}"
  out_dir="/app/component/wac/deps/component"

  log "pulling WAC deps into $out_dir"
  log "- eventbus:  $eventbus_ref"
  log "- scheduler: $scheduler_ref"

  set +e
  HARBOR_REGISTRY="$registry" \
  HARBOR_REF="$eventbus_ref" \
  OUTPUT_DIR="$out_dir" \
  /usr/local/bin/ntx-oras-pull.sh
  rc1=$?

  HARBOR_REGISTRY="$registry" \
  HARBOR_REF="$scheduler_ref" \
  OUTPUT_DIR="$out_dir" \
  /usr/local/bin/ntx-oras-pull.sh
  rc2=$?
  set -e

  if [[ $rc1 -ne 0 || $rc2 -ne 0 ]]; then
    log "warning: failed to pull WAC deps from Harbor (rc_eventbus=$rc1 rc_scheduler=$rc2); backend will still start"
    log "hint: set HARBOR_CA_FILE/HARBOR_USER/HARBOR_PASS or ORAS_INSECURE=1 (self-signed), ORAS_PLAIN_HTTP=1 (http), and ensure Harbor is reachable"
  fi
fi

# Ensure repo-root marker exists (used by auto-detect if wac_compose_cwd not set)
[[ -f /app/component/wac/scheduler-composition.wac ]] || die "missing /app/component/wac/scheduler-composition.wac"

log "starting ntx-backend"
exec /usr/local/bin/ntx-backend --config "$CONFIG_PATH"
