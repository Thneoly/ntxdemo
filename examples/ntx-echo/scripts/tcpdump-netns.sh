#!/usr/bin/env bash
set -euo pipefail

# tcpdump capture inside the ntx-echo netns topology.
#
# Default: capture in netns ntxns1 on iface ntx1 and write a timestamped pcap.
#
# Usage:
#   sudo ./examples/ntx-echo/scripts/tcpdump-netns.sh
#
# Env overrides:
#   NS=ntxns1 IFACE=ntx1 OUT=target/ntx-echo/netns.pcap FILTER='arp or (udp and port 7)'

NS="${NS:-ntxns1}"
IFACE="${IFACE:-ntx1}"
FILTER="${FILTER:-arp or (udp and port 7)}"
DIR="${DIR:-inout}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

mkdir -p target/ntx-echo
stamp="$(date +%Y%m%d-%H%M%S)"
OUT="${OUT:-target/ntx-echo/netns-${NS}-${IFACE}-${stamp}.pcap}"

if [[ ${EUID} -ne 0 ]]; then
  echo "This script must be run as root." >&2
  exit 1
fi

if ! ip netns list | awk '{print $1}' | grep -qx "${NS}"; then
  echo "netns '${NS}' not found. Bring up topology first:" >&2
  echo "  sudo ./scripts/ntx-veth-up.sh" >&2
  exit 1
fi

if ! ip -n "${NS}" link show "${IFACE}" >/dev/null 2>&1; then
  echo "iface '${IFACE}' not found in netns '${NS}'." >&2
  exit 1
fi

echo "[tcpdump] netns=${NS} iface=${IFACE} out=${OUT} filter=${FILTER}"
echo "[tcpdump] direction=${DIR} (set DIR=in|out|inout)"
echo "Stop with Ctrl-C"

exec ip netns exec "${NS}" tcpdump -ni "${IFACE}" -Q "${DIR}" -s 0 -w "${OUT}" ${FILTER}
