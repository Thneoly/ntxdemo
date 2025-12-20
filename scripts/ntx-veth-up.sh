#!/usr/bin/env bash
set -euo pipefail

# Create a veth pair (ntx0 <-> ntx1) and a network namespace (ntxns1) for same-host L2 validation.
#
# Topology:
#   host namespace:  ntx0  (L2 only by default)
#   netns ntxns1:    ntx1  (L2 only by default)
#
# This lets you run our AF_PACKET-based examples on both sides without using a physical NIC.
#
# Notes:
# - For AF_PACKET / raw L2 testing, IP addresses are NOT required.
# - If you want optional kernel-stack L3 validation (ping/ARP), set ENABLE_IP=1.

NS="ntxns1"
IF_HOST="ntx0"
IF_NS="ntx1"
IP_HOST="10.0.0.1/24"
IP_NS="10.0.0.2/24"
ENABLE_IP="${ENABLE_IP:-0}"

if [[ $EUID -ne 0 ]]; then
  echo "This script must be run as root." >&2
  exit 1
fi

# Clean up any previous run to make it idempotent.
if ip netns list | grep -q "^${NS} "; then
  ip netns del "${NS}" || true
fi
if ip link show "${IF_HOST}" >/dev/null 2>&1; then
  ip link del "${IF_HOST}" || true
fi

# Create netns + veth.
ip netns add "${NS}"
ip link add "${IF_HOST}" type veth peer name "${IF_NS}"

# Move peer into netns.
ip link set "${IF_NS}" netns "${NS}"

# Bring up host side.
ip link set "${IF_HOST}" up

# Bring up netns side.
ip -n "${NS}" link set lo up
ip -n "${NS}" link set "${IF_NS}" up

if [[ "${ENABLE_IP}" == "1" ]]; then
  # Optional L3 config for kernel-stack validation.
  ip addr add "${IP_HOST}" dev "${IF_HOST}" || true
  ip -n "${NS}" addr add "${IP_NS}" dev "${IF_NS}" || true
fi

# Optional: show summary.
echo "[ok] created veth pair and netns"
if [[ "${ENABLE_IP}" == "1" ]]; then
  echo "- host:  ${IF_HOST}  ${IP_HOST}"
  echo "- netns: ${NS}:${IF_NS}  ${IP_NS}"
else
  echo "- host:  ${IF_HOST}  (no IP; L2 only)"
  echo "- netns: ${NS}:${IF_NS}  (no IP; L2 only)"
fi

if [[ "${ENABLE_IP}" == "1" ]]; then
  echo "Quick ping check (ICMP via kernel stack, optional):"
  ping -c 1 -W 1 10.0.0.2 >/dev/null && echo "- host -> netns ping ok" || echo "- host -> netns ping failed (may still be fine for raw L2 tests)"
  ip netns exec "${NS}" ping -c 1 -W 1 10.0.0.1 >/dev/null && echo "- netns -> host ping ok" || echo "- netns -> host ping failed (may still be fine for raw L2 tests)"
else
  echo "IP config is disabled (ENABLE_IP=0). For ping/ARP/kernel L3 checks, re-run with ENABLE_IP=1."
fi
