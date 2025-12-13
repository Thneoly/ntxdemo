#!/usr/bin/env bash
set -euo pipefail

# Run a command inside the ntxns1 network namespace.
# Usage: sudo ./scripts/ntxns1.sh <command> [args...]

NS="ntxns1"

if [[ $EUID -ne 0 ]]; then
  echo "This script must be run as root (ip netns exec requires it)." >&2
  exit 1
fi

if ! ip netns list | grep -q "^${NS} "; then
  echo "Namespace ${NS} not found. Run scripts/ntx-veth-up.sh first." >&2
  exit 1
fi

exec ip netns exec "${NS}" "$@"
