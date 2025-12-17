#!/usr/bin/env bash
set -euo pipefail

# Run the ntx-echo server inside the netns topology created by scripts/ntx-veth-up.sh.
#
# Defaults match scripts/ntx-veth-up.sh:
#   netns:  ntxns1
#   iface:  ntx1
#   ip:     10.0.0.2
#   udp:    7
#
# Overrides via env:
#   NS=ntxns1 IFACE=ntx1 PROFILE=debug
#
# Usage:
#   ./examples/ntx-echo/scripts/run-server.sh
#
# It will:
#   1) build as the current user
#   2) re-exec itself via sudo to run the produced binary inside netns

NS="${NS:-ntxns1}"
IFACE="${IFACE:-ntx1}"
PROFILE="${PROFILE:-debug}"
EXAMPLE="ntx-echo-server"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

# Stage 1: build as the current user.
if [[ ${EUID} -ne 0 ]]; then
  echo "[build] cargo build --example ${EXAMPLE} (${PROFILE})"
  if [[ "${PROFILE}" == "release" ]]; then
    cargo build -q --example "${EXAMPLE}" --release
  else
    cargo build -q --example "${EXAMPLE}"
  fi

  echo "[re-exec] sudo $0 (run in netns)"
  exec sudo --preserve-env=NS,IFACE,PROFILE "$0"
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

echo "[run] ip netns exec ${NS} ${bin_path} ${IFACE}"
exec ip netns exec "${NS}" "${bin_path}" "${IFACE}"
