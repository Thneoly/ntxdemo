#!/usr/bin/env bash
set -euo pipefail

# Run the ntx-echo server inside the netns topology created by scripts/ntx-veth-up.sh.
#
# Defaults match scripts/ntx-veth-up.sh:
#   netns:  ntxns1
#   iface:  ntx1
#   ip:     10.0.0.2 and 10.0.0.3 (from resource pools)
#   udp:    7
#
# Overrides via env:
#   NS=ntxns1 IFACE=ntx1 PROFILE=debug
#   RESOURCES_YAML=./resources.yaml TARGETS_YAML=./targets.yaml
#
# CLI:
#   ./examples/ntx-echo/scripts/run-server.sh [iface] [resources.yaml] [targets.yaml]
#
# Precedence (highest to lowest):
#   1) CLI args
#   2) env vars
#   3) defaults
#
# It will:
#   1) build as the current user
#   2) re-exec itself via sudo to run the produced binary inside netns

NS="${NS:-ntxns1}"
IFACE_DEFAULT="${IFACE:-ntx1}"
PROFILE="${PROFILE:-debug}"
EXAMPLE="ntx-echo-server"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

DEFAULT_RESOURCES_YAML="${repo_root}/examples/ntx-echo/resource/server.yaml"
DEFAULT_TARGETS_YAML="${repo_root}/examples/ntx-echo/resource/targets.yaml"

usage() {
  cat <<EOF
Usage:
  $0 [iface] [resources.yaml] [targets.yaml]

Examples:
  $0
  $0 ntx1
  $0 ntx1 ./resources.yaml

Environment:
  NS              netns name (default: ntxns1)
  IFACE           Default iface if no CLI arg given (default: ntx1)
  PROFILE         debug|release (default: debug)
  RESOURCES_YAML  Default resources yaml if no CLI arg given
  TARGETS_YAML    Optional targets yaml (used to infer identity count)

Default resources file:
  ${DEFAULT_RESOURCES_YAML}

Default targets file:
  ${DEFAULT_TARGETS_YAML}
EOF
}

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
esac

IFACE="${1:-${IFACE_DEFAULT}}"

# Optional argv[2] for the server: resource pool YAML file.
# If empty/unset, the server will keep using its legacy fixed identity.
RESOURCES_YAML="${2:-${RESOURCES_YAML:-${DEFAULT_RESOURCES_YAML}}}"

# Optional argv[3] for the server: targets yaml file.
TARGETS_YAML="${3:-${TARGETS_YAML:-${DEFAULT_TARGETS_YAML}}}"

# Allow disabling yaml by passing '-' as argv[3].
if [[ "${TARGETS_YAML}" == "-" ]]; then
  TARGETS_YAML=""
fi

# Allow disabling yaml by passing '-' as argv[2].
if [[ "${RESOURCES_YAML}" == "-" ]]; then
  RESOURCES_YAML=""
fi

# Stage 1: build as the current user.
if [[ ${EUID} -ne 0 ]]; then
  echo "[build] cargo build --example ${EXAMPLE} (${PROFILE})"
  if [[ "${PROFILE}" == "release" ]]; then
    cargo build -q --example "${EXAMPLE}" --release
  else
    cargo build -q --example "${EXAMPLE}"
  fi

  echo "[re-exec] sudo $0 (run in netns)"
  exec sudo --preserve-env=NS,IFACE,PROFILE,RESOURCES_YAML,TARGETS_YAML "$0" "${IFACE}" "${RESOURCES_YAML}" "${TARGETS_YAML}"
fi

# Stage 2: run as root.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

if ! ip netns list | awk '{print $1}' | grep -qx "${NS}"; then
  echo "netns '${NS}' not found. Bring up topology first:" >&2
  echo "  sudo ./scripts/ntx-veth-up.sh" >&2
  exit 1
fi

if ! ip -n "${NS}" link show "${IFACE}" >/dev/null 2>&1; then
  echo "iface '${IFACE}' not found in netns '${NS}'." >&2
  exit 1
fi

# Build on host (more reliable than running cargo inside netns where PATH may differ).
if [[ "${PROFILE}" == "release" ]]; then
  bin_path="${repo_root}/target/release/examples/${EXAMPLE}"
else
  bin_path="${repo_root}/target/debug/examples/${EXAMPLE}"
fi

if [[ ! -x "${bin_path}" ]]; then
  echo "built binary not found: ${bin_path}" >&2
  exit 1
fi

if [[ -n "${RESOURCES_YAML}" ]]; then
  if [[ ! -f "${RESOURCES_YAML}" ]]; then
    echo "resources yaml not found: ${RESOURCES_YAML}" >&2
    echo "hint: pass '-' as argv[2] to disable yaml and use the legacy fixed identity" >&2
    exit 1
  fi
  if [[ -n "${TARGETS_YAML}" ]]; then
    if [[ ! -f "${TARGETS_YAML}" ]]; then
      echo "targets yaml not found: ${TARGETS_YAML}" >&2
      echo "hint: pass '-' as argv[3] to disable targets yaml" >&2
      exit 1
    fi
    echo "[run] ip netns exec ${NS} ${bin_path} ${IFACE} ${RESOURCES_YAML} ${TARGETS_YAML}"
    exec ip netns exec "${NS}" "${bin_path}" "${IFACE}" "${RESOURCES_YAML}" "${TARGETS_YAML}"
  else
    echo "[run] ip netns exec ${NS} ${bin_path} ${IFACE} ${RESOURCES_YAML}"
    exec ip netns exec "${NS}" "${bin_path}" "${IFACE}" "${RESOURCES_YAML}"
  fi
else
  echo "[run] ip netns exec ${NS} ${bin_path} ${IFACE}"
  exec ip netns exec "${NS}" "${bin_path}" "${IFACE}"
fi
