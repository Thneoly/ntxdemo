#!/usr/bin/env bash
set -euo pipefail

# Capture both sides of the ntx-echo topology into two pcaps.
#
# - Host namespace: IFACE_HOST (default: ntx0)
# - Netns namespace: NS (default: ntxns1), IFACE_NS (default: ntx1)
#
# Output directory: ./target/ntx-echo/
#   host-<iface>-<stamp>.pcap
#   netns-<ns>-<iface>-<stamp>.pcap
#
# Usage:
#   sudo ./examples/ntx-echo/scripts/tcpdump-bidir.sh
#
# Env overrides:
#   IFACE_HOST=ntx0 NS=ntxns1 IFACE_NS=ntx1
#   FILTER='arp or (udp and port 7)'
#   OUT_DIR=target/ntx-echo

IFACE_HOST="${IFACE_HOST:-ntx0}"
NS="${NS:-ntxns1}"
IFACE_NS="${IFACE_NS:-ntx1}"
FILTER="${FILTER:-arp or (udp and port 7)}"
DIR="${DIR:-inout}"
OUT_DIR="${OUT_DIR:-target/ntx-echo}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

if [[ ${EUID} -ne 0 ]]; then
  echo "This script must be run as root." >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"
stamp="$(date +%Y%m%d-%H%M%S)"

OUT_HOST="${OUT_DIR}/host-${IFACE_HOST}-${stamp}.pcap"
OUT_NS="${OUT_DIR}/netns-${NS}-${IFACE_NS}-${stamp}.pcap"

if ! ip link show "${IFACE_HOST}" >/dev/null 2>&1; then
  echo "host iface '${IFACE_HOST}' not found." >&2
  exit 1
fi

if ! ip netns list | awk '{print $1}' | grep -qx "${NS}"; then
  echo "netns '${NS}' not found. Bring up topology first:" >&2
  echo "  sudo ./scripts/ntx-veth-up.sh" >&2
  exit 1
fi

if ! ip -n "${NS}" link show "${IFACE_NS}" >/dev/null 2>&1; then
  echo "iface '${IFACE_NS}' not found in netns '${NS}'." >&2
  exit 1
fi

echo "[tcpdump] host iface=${IFACE_HOST} out=${OUT_HOST} filter=${FILTER}"
echo "[tcpdump] netns=${NS} iface=${IFACE_NS} out=${OUT_NS} filter=${FILTER}"
echo "[tcpdump] direction=${DIR} (set DIR=in|out|inout)"
echo "Stop with Ctrl-C (both captures will be terminated)"

# Ensure both tcpdump processes are cleaned up on exit.
cleanup() {
  # best-effort
  if [[ -n "${PID_HOST:-}" ]]; then kill "${PID_HOST}" >/dev/null 2>&1 || true; fi
  if [[ -n "${PID_NS:-}" ]]; then kill "${PID_NS}" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT INT TERM

# Host capture
( exec tcpdump -ni "${IFACE_HOST}" -Q "${DIR}" -s 0 -w "${OUT_HOST}" ${FILTER} ) &
PID_HOST=$!

# Netns capture
( exec ip netns exec "${NS}" tcpdump -ni "${IFACE_NS}" -Q "${DIR}" -s 0 -w "${OUT_NS}" ${FILTER} ) &
PID_NS=$!

# Wait until one exits (Ctrl-C triggers trap).
wait -n "${PID_HOST}" "${PID_NS}" || true
