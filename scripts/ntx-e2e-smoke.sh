#!/usr/bin/env bash
set -euo pipefail

# ntx-e2e-smoke.sh
# A lightweight smoke/runbook helper for the veth+netns UDP echo + traffic-send setup.
#
# Scope:
# - Does NOT create or modify the topology automatically (to avoid surprises).
# - Only checks whether ntx0/ntx1/ntxns1 exist and prints copy/paste commands.

have_cmd() { command -v "$1" >/dev/null 2>&1; }

need_tools=(ip)
for t in "${need_tools[@]}"; do
  if ! have_cmd "$t"; then
    echo "missing required tool: $t" >&2
    exit 1
  fi
done

have_link() {
  ip link show dev "$1" >/dev/null 2>&1
}

have_ns() {
  ip netns list | awk '{print $1}' | grep -qx "$1"
}

echo "== ntx e2e smoke (veth+netns) =="

ok=true
if have_link ntx0; then
  echo "[ok] link ntx0 exists"
else
  echo "[!!] link ntx0 missing"
  ok=false
fi

if have_ns ntxns1; then
  echo "[ok] netns ntxns1 exists"
else
  echo "[!!] netns ntxns1 missing"
  ok=false
fi

if $ok; then
  # ntx1 is inside the netns in our scripts; check it there.
  if ip netns exec ntxns1 ip link show dev ntx1 >/dev/null 2>&1; then
    echo "[ok] link ntx1 exists in netns ntxns1"
  else
    echo "[!!] link ntx1 missing in netns ntxns1"
    ok=false
  fi
fi

echo
if ! $ok; then
  cat <<'EOF'
Topology not ready.

Create it with:
  sudo ./scripts/ntx-veth-up.sh

Cleanup with:
  sudo ./scripts/ntx-veth-down.sh
EOF
  echo
fi

cat <<'EOF'
== Build (run as normal user) ==
  cargo build --example userspace-udp-echo
  cargo build --example traffic-send

== Runbook (3 terminals) ==

Terminal A (host echo):
  sudo ./target/debug/examples/userspace-udp-echo --iface ntx0 --backend tpacketv3 --port 10001 --verbose

Terminal B (netns echo):
  sudo ./scripts/ntxns1.sh ./target/debug/examples/userspace-udp-echo --iface ntx1 --backend tpacketv3 --port 10001 --verbose

Terminal C (client host -> netns):
  sudo ./target/debug/examples/traffic-send --iface ntx0 --backend tpacketv3 --dst-ips 10.0.0.2 --src-ip 10.0.0.1 --dst-port 10001 --src-port 40000 --rr --arp --pps 50 --count 50

Optional tcpdump (host):
  sudo tcpdump -ni ntx0 -vv -e 'arp or (udp and (port 10001 or port 40000))'
EOF
