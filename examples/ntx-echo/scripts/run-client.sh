#!/usr/bin/env bash
set -euo pipefail

# Run the ntx-echo client on the host side of the topology created by scripts/ntx-veth-up.sh.
#
# Defaults match scripts/ntx-veth-up.sh:
#   iface: ntx0
#
# Overrides via env:
#   IFACE=ntx0 PROFILE=debug
#   CLIENT_YAML=./client.yaml TARGETS_YAML=./targets.yaml
#
# CLI:
#   ./examples/ntx-echo/scripts/run-client.sh [iface] [client.yaml] [targets.yaml]
#
# Precedence (highest to lowest):
#   1) CLI args
#   2) env vars
#   3) defaults
#
# It will:
#   1) build as the current user
#   2) re-exec itself via sudo to run the produced binary

IFACE_DEFAULT="${IFACE:-ntx0}"
PROFILE="${PROFILE:-debug}"
EXAMPLE="ntx-echo-client"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

DEFAULT_CLIENT_YAML="${repo_root}/examples/ntx-echo/resource/client.yaml"
DEFAULT_TARGETS_YAML="${repo_root}/examples/ntx-echo/resource/targets.yaml"

usage() {
  cat <<EOF
Usage:
  $0 [iface] [client.yaml] [targets.yaml]

Examples:
  $0
  $0 ntx0
  $0 ntx0 ./client.yaml ./targets.yaml

Environment:
  IFACE        Default iface if no CLI arg given (default: ntx0)
  PROFILE      debug|release (default: debug)
  CLIENT_YAML  Default client yaml if no CLI arg given
  TARGETS_YAML Default targets yaml if no CLI arg given ('-' to disable and ARP-resolve)

Default yaml files:
  client:  ${DEFAULT_CLIENT_YAML}
  targets: ${DEFAULT_TARGETS_YAML}
EOF
}

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
esac

# CLI overrides env overrides defaults.
IFACE="${1:-${IFACE_DEFAULT}}"
CLIENT_YAML="${2:-${CLIENT_YAML:-${DEFAULT_CLIENT_YAML}}}"
TARGETS_YAML="${3:-${TARGETS_YAML:-${DEFAULT_TARGETS_YAML}}}"

# Allow disabling either yaml by passing '-'.
if [[ "${CLIENT_YAML}" == "-" ]]; then
  CLIENT_YAML=""
fi
if [[ "${TARGETS_YAML}" == "-" ]]; then
  TARGETS_YAML=""
fi

# Best-effort check that the veth exists.
if ! ip link show "${IFACE}" >/dev/null 2>&1; then
  echo "iface '${IFACE}' not found. Bring up topology first:" >&2
  echo "  sudo ./scripts/ntx-veth-up.sh" >&2
  exit 1
fi

# Stage 1: build as the current user (cargo/rustup config lives in the user account).
if [[ ${EUID} -ne 0 ]]; then
  echo "[build] cargo build --example ${EXAMPLE} (${PROFILE})"
  if [[ "${PROFILE}" == "release" ]]; then
    cargo build -q --example "${EXAMPLE}" --release
  else
    cargo build -q --example "${EXAMPLE}"
  fi

  echo "[re-exec] sudo $0 (run binary with CAP_NET_RAW)"
  exec sudo --preserve-env=IFACE,PROFILE,CLIENT_YAML,TARGETS_YAML "$0" \
    "${IFACE}" "${CLIENT_YAML}" "${TARGETS_YAML}"
fi

# Stage 2: run as root.

if [[ "${PROFILE}" == "release" ]]; then
  bin_path="${repo_root}/target/release/examples/${EXAMPLE}"
else
  bin_path="${repo_root}/target/debug/examples/${EXAMPLE}"
fi

if [[ ! -x "${bin_path}" ]]; then
  echo "built binary not found: ${bin_path}" >&2
  exit 1
fi

if [[ -z "${CLIENT_YAML}" ]]; then
  echo "client yaml is required (pass path as argv[2])" >&2
  exit 1
fi
if [[ ! -f "${CLIENT_YAML}" ]]; then
  echo "client yaml not found: ${CLIENT_YAML}" >&2
  exit 1
fi

if [[ -n "${TARGETS_YAML}" && ! -f "${TARGETS_YAML}" ]]; then
  echo "targets yaml not found: ${TARGETS_YAML}" >&2
  exit 1
fi

if [[ -n "${TARGETS_YAML}" ]]; then
  echo "[run] ${bin_path} ${IFACE} ${CLIENT_YAML} ${TARGETS_YAML}"
  exec "${bin_path}" "${IFACE}" "${CLIENT_YAML}" "${TARGETS_YAML}"
else
  echo "[run] ${bin_path} ${IFACE} ${CLIENT_YAML}"
  exec "${bin_path}" "${IFACE}" "${CLIENT_YAML}"
fi
