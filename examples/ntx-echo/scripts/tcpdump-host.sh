#!/usr/bin/env bash
set -euo pipefail

# tcpdump capture on host on the ntx-echo topology.
#
# Default: capture on ntx0 and write a timestamped pcap under ./target/ntx-echo/
#
# Usage:
#   sudo ./examples/ntx-echo/scripts/tcpdump-host.sh
#
# Env overrides:
#   IFACE=ntx0 OUT=target/ntx-echo/host.pcap FILTER='arp or (udp and port 7)'

IFACE="${IFACE:-ntx0}"
FILTER="${FILTER:-arp or (udp and port 7)}"
DIR="${DIR:-inout}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

mkdir -p target/ntx-echo
stamp="$(date +%Y%m%d-%H%M%S)"
OUT="${OUT:-target/ntx-echo/host-${IFACE}-${stamp}.pcap}"

if [[ ${EUID} -ne 0 ]]; then
  echo "This script must be run as root." >&2
  exit 1
fi

if ! ip link show "${IFACE}" >/dev/null 2>&1; then
  echo "iface '${IFACE}' not found." >&2
  exit 1
fi

echo "[tcpdump] iface=${IFACE} out=${OUT} filter=${FILTER}"
echo "[tcpdump] direction=${DIR} (set DIR=in|out|inout)"
echo "Stop with Ctrl-C"

exec tcpdump -Z root -ni "${IFACE}" -Q "${DIR}" -s 0 -w "${OUT}" ${FILTER}
