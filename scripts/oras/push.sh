#!/bin/bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"

# Required/optional env vars:
#   HARBOR_REGISTRY   e.g. 192.168.31.138 or harbor.example.com:8443
#   HARBOR_REF        e.g. 192.168.31.138/ntx/executor:v0.0.1
#   HARBOR_USER       (optional) registry username
#   HARBOR_PASS       (optional) registry password (used with --password-stdin)
#   HARBOR_CA_FILE    (optional) CA cert for self-signed registry
#   ORAS_INSECURE=1   (optional) pass --insecure
#   ORAS_PLAIN_HTTP=1 (optional) pass --plain-http
#   ARTIFACT_TYPE     (optional) default: application/vnd.ntx.action-executor.v1
#   OUTPUT_DIR        (optional) directory to place generated/picked artifacts before pushing
#                    default: scripts/oras/
#   PUSH_WASM_ONLY=1  (optional) push only the wasm file (no catalog json)
#
# Build / inputs:
#   SKIP_BUILD=1      skip cargo build & copy
#   WASM_PATH         when SKIP_BUILD=1, path to wasm to publish
#
# Outputs:
#   scripts/oras/actions_executor.wasm
#   scripts/oras/actions-catalog.json

if [[ -z "${HARBOR_REF:-}" ]]; then
  echo "ERROR: HARBOR_REF is required." >&2
  echo "Example:" >&2
  echo "  export HARBOR_REGISTRY=192.168.31.138" >&2
  echo "  export HARBOR_REF=\"$HARBOR_REGISTRY/ntx/executor:v0.0.1\"" >&2
  echo "  export HARBOR_CA_FILE=/home/cc/Desktop/harbor/certs/harbor.crt" >&2
  echo "  export HARBOR_USER=admin" >&2
  echo "  export HARBOR_PASS='***'" >&2
  echo "  $0" >&2
  exit 2
fi

artifact_type="${ARTIFACT_TYPE:-application/vnd.ntx.action-executor.v1}"

output_dir="${OUTPUT_DIR:-$script_dir}"
mkdir -p "$output_dir"

out_wasm="$output_dir/actions_executor.wasm"
out_catalog="$output_dir/actions-catalog.json"

if [[ "${SKIP_BUILD:-}" == "1" ]]; then
  if [[ -z "${WASM_PATH:-}" ]]; then
    echo "ERROR: SKIP_BUILD=1 requires WASM_PATH=/path/to/actions_executor.wasm" >&2
    exit 2
  fi

  # Preserve filename so callers can push eventbus.wasm / scheduler.wasm, etc.
  out_wasm="$output_dir/$(basename "$WASM_PATH")"
  cp "$WASM_PATH" "$out_wasm"
else
  pushd "$repo_root" >/dev/null
    cargo build -p actions-executor --target wasm32-wasip2
    if [[ ! -f target/wasm32-wasip2/debug/actions_executor.wasm ]]; then
      echo "ERROR: build succeeded but target/wasm32-wasip2/debug/actions_executor.wasm not found" >&2
      exit 1
    fi
    cp target/wasm32-wasip2/debug/actions_executor.wasm "$out_wasm"
  popd >/dev/null
fi

if [[ "${PUSH_WASM_ONLY:-}" != "1" ]]; then
  pushd "$repo_root" >/dev/null
    cargo run -p actions-catalog-gen -- "$out_wasm" "$out_catalog"
  popd >/dev/null
fi

login_args=()
push_args=()

if [[ -n "${HARBOR_CA_FILE:-}" ]]; then
  login_args+=(--ca-file "$HARBOR_CA_FILE")
  push_args+=(--ca-file "$HARBOR_CA_FILE")
fi
if [[ "${ORAS_INSECURE:-}" == "1" ]]; then
  login_args+=(--insecure)
  push_args+=(--insecure)
fi
if [[ "${ORAS_PLAIN_HTTP:-}" == "1" ]]; then
  login_args+=(--plain-http)
  push_args+=(--plain-http)
fi

if [[ -n "${HARBOR_USER:-}" ]]; then
  if [[ -n "${HARBOR_PASS:-}" ]]; then
    printf '%s' "$HARBOR_PASS" | oras login "${login_args[@]}" -u "$HARBOR_USER" --password-stdin "${HARBOR_REGISTRY:-${HARBOR_REF%%/*}}"
  else
    oras login "${login_args[@]}" -u "$HARBOR_USER" "${HARBOR_REGISTRY:-${HARBOR_REF%%/*}}"
  fi
fi

if [[ "${PUSH_WASM_ONLY:-}" == "1" ]]; then
  pushd "$output_dir" >/dev/null
    oras push "${push_args[@]}" "$HARBOR_REF" \
      --artifact-type "$artifact_type" \
      "$(basename "$out_wasm"):application/wasm"
  popd >/dev/null
else
  pushd "$output_dir" >/dev/null
    oras push "${push_args[@]}" "$HARBOR_REF" \
      --artifact-type "$artifact_type" \
      "$(basename "$out_wasm"):application/wasm" \
      "$(basename "$out_catalog"):application/json"
  popd >/dev/null
fi