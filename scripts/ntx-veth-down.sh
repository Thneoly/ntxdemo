#!/usr/bin/env bash
set -euo pipefail

NS="ntxns1"
IF_HOST="ntx0"

if [[ $EUID -ne 0 ]]; then
  echo "This script must be run as root." >&2
  exit 1
fi

# Deleting the netns also deletes interfaces moved into it.
if ip netns list | grep -q "^${NS} "; then
  ip netns del "${NS}" || true
fi

# Delete host-side veth if still present.
if ip link show "${IF_HOST}" >/dev/null 2>&1; then
  ip link del "${IF_HOST}" || true
fi

echo "[ok] removed ${IF_HOST} and netns ${NS}"