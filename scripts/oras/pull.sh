#!/bin/bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Required/optional env vars:
#   HARBOR_REGISTRY   e.g. 192.168.31.138 or harbor.example.com:8443
#   HARBOR_REF        e.g. 192.168.31.138/ntx/executor:v0.0.1
#   HARBOR_USER       (optional) registry username
#   HARBOR_PASS       (optional) registry password (used with --password-stdin)
#   HARBOR_CA_FILE    (optional) CA cert for self-signed registry
#   ORAS_INSECURE=1   (optional) pass --insecure
#   ORAS_PLAIN_HTTP=1 (optional) pass --plain-http
#   OUTPUT_DIR        (optional) output directory; default: scripts/oras/tmp

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

output_dir="${OUTPUT_DIR:-$script_dir/tmp}"
mkdir -p "$output_dir"

login_args=()
pull_args=()

if [[ -n "${HARBOR_CA_FILE:-}" ]]; then
  login_args+=(--ca-file "$HARBOR_CA_FILE")
  pull_args+=(--ca-file "$HARBOR_CA_FILE")
fi
if [[ "${ORAS_INSECURE:-}" == "1" ]]; then
  login_args+=(--insecure)
  pull_args+=(--insecure)
fi
if [[ "${ORAS_PLAIN_HTTP:-}" == "1" ]]; then
  login_args+=(--plain-http)
  pull_args+=(--plain-http)
fi

# Login is optional: if you already logged in (oras credential store), you can omit HARBOR_USER/PASS.
if [[ -n "${HARBOR_USER:-}" ]]; then
  if [[ -n "${HARBOR_PASS:-}" ]]; then
    printf '%s' "$HARBOR_PASS" | oras login "${login_args[@]}" -u "$HARBOR_USER" --password-stdin "${HARBOR_REGISTRY:-${HARBOR_REF%%/*}}"
  else
    oras login "${login_args[@]}" -u "$HARBOR_USER" "${HARBOR_REGISTRY:-${HARBOR_REF%%/*}}"
  fi
fi

oras pull "${pull_args[@]}" "$HARBOR_REF" -o "$output_dir"