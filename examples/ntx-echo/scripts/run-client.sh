#!/usr/bin/env bash
set -euo pipefail

# Run the ntx-echo client on the host side of the topology created by scripts/ntx-veth-up.sh.
#
# Defaults match scripts/ntx-veth-up.sh:
#   iface: ntx0
#
# Overrides via env:
#   IFACE=ntx0 PROFILE=debug
#
# Usage:
#   ./examples/ntx-echo/scripts/run-client.sh
#
# It will:
#   1) build as the current user
#   2) re-exec itself via sudo to run the produced binary

IFACE="${IFACE:-ntx0}"
PROFILE="${PROFILE:-debug}"
EXAMPLE="ntx-echo-client"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

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
  exec sudo --preserve-env=IFACE,PROFILE "$0"
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

echo "[run] ${bin_path} ${IFACE}"
exec "${bin_path}" "${IFACE}"
