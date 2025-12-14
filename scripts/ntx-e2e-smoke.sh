#!/usr/bin/env bash
set -euo pipefail

# ntx-e2e-smoke.sh
# A lightweight smoke/runbook helper for the veth+netns UDP echo + traffic-send setup.
#
# Scope:
# - Does NOT create or modify the topology automatically (to avoid surprises).
# - Only checks whether ntx0/ntx1/ntxns1 exist and prints copy/paste commands.

usage() {
  cat <<'EOF'
ntx-e2e-smoke.sh

Default: print-only runbook + topology checks.

Options:
  --run              Run a short end-to-end check automatically (Host Ntx net-mode + netns traffic-send).
  --timeout <secs>   Overall timeout for the run mode (default: 8).
  --count <n>        traffic-send packet count (default: 50).
  --pps <n>          traffic-send pps (default: 50).
  --backend <name>   NIC backend: afpacket|afpacket-dgram|tpacketv3 (default: afpacket).
  --iface <name>     Host iface (default: ntx0).
  --ns <name>        Netns name (default: ntxns1).
  --ns-iface <name>  Netns iface inside ns (default: ntx1).
  --port <n>         UDP dst port (default: 10001).
  --host-ip <ip>     Host IP used by traffic-send (default: 10.0.0.1).
  --ns-ip <ip>       Netns IP used by traffic-send (default: 10.0.0.2).
  --build            Build required binaries before running.
  --tcpdump          Also run tcpdump on host iface during --run.
  -h, --help         Show this help.

Examples:
  ./scripts/ntx-e2e-smoke.sh
  ./scripts/ntx-e2e-smoke.sh --run
  ./scripts/ntx-e2e-smoke.sh --run --timeout 12 --count 200 --pps 100
  ./scripts/ntx-e2e-smoke.sh --run --tcpdump --timeout 6 --count 10 --pps 50
  ./scripts/ntx-e2e-smoke.sh --run --backend tpacketv3 --tcpdump
EOF
}

RUN=false
DO_BUILD=false
DO_TCPDUMP=false
TIMEOUT_SECS=8
COUNT=50
PPS=50
BACKEND=afpacket
IFACE_HOST=ntx0
NS=ntxns1
IFACE_NS=ntx1
PORT=10001
HOST_IP=10.0.0.1
NS_IP=10.0.0.2

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) RUN=true; shift ;;
    --build) DO_BUILD=true; shift ;;
    --tcpdump) DO_TCPDUMP=true; shift ;;
    --timeout) TIMEOUT_SECS="${2:?missing value}"; shift 2 ;;
    --count) COUNT="${2:?missing value}"; shift 2 ;;
    --pps) PPS="${2:?missing value}"; shift 2 ;;
    --backend) BACKEND="${2:?missing value}"; shift 2 ;;
    --iface) IFACE_HOST="${2:?missing value}"; shift 2 ;;
    --ns) NS="${2:?missing value}"; shift 2 ;;
    --ns-iface) IFACE_NS="${2:?missing value}"; shift 2 ;;
    --port) PORT="${2:?missing value}"; shift 2 ;;
    --host-ip) HOST_IP="${2:?missing value}"; shift 2 ;;
    --ns-ip) NS_IP="${2:?missing value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "unknown argument: $1" >&2
      echo
      usage >&2
      exit 2
      ;;
  esac
done

have_cmd() { command -v "$1" >/dev/null 2>&1; }

need_tools=(ip)
for t in "${need_tools[@]}"; do
  if ! have_cmd "$t"; then
    echo "missing required tool: $t" >&2
    exit 1
  fi
done

have_link() { ip link show dev "$1" >/dev/null 2>&1; }

have_ns() { ip netns list | awk '{print $1}' | grep -qx "$1"; }

echo "== ntx e2e smoke (veth+netns) =="

ok=true
if have_link "${IFACE_HOST}"; then
  echo "[ok] link ${IFACE_HOST} exists"
else
  echo "[!!] link ${IFACE_HOST} missing"
  ok=false
fi

if have_ns "${NS}"; then
  echo "[ok] netns ${NS} exists"
else
  echo "[!!] netns ${NS} missing"
  ok=false
fi

if $ok; then
  # ntx1 is inside the netns in our scripts; check it there.
  if ip netns exec "${NS}" ip link show dev "${IFACE_NS}" >/dev/null 2>&1; then
    echo "[ok] link ${IFACE_NS} exists in netns ${NS}"
  else
    echo "[!!] link ${IFACE_NS} missing in netns ${NS}"
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

if $RUN; then
  echo "== auto run mode =="

  if [[ $EUID -ne 0 ]]; then
    echo "[!!] --run needs root (raw sockets). Please re-run with sudo." >&2
    exit 1
  fi

  if ! $ok; then
    echo "[!!] topology not ready; refusing to run automatically." >&2
    exit 1
  fi

  # Optional build. We keep it explicit because build can be slow and may need toolchains.
  if $DO_BUILD; then
    echo "[..] building binaries (cargo build + examples)"
    (cd "$(dirname "${BASH_SOURCE[0]}")/.." && cargo build && cargo build --example traffic-send)
  fi

  ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  BIN_NTX="${ROOT}/target/debug/Ntx"
  BIN_TRAFFIC="${ROOT}/target/debug/examples/traffic-send"
  NS_WRAP="${ROOT}/scripts/ntxns1.sh"

  if [[ ! -x "${BIN_NTX}" ]]; then
    echo "[!!] missing ${BIN_NTX}. Build it first (cargo build) or use --build." >&2
    exit 1
  fi
  if [[ ! -x "${BIN_TRAFFIC}" ]]; then
    echo "[!!] missing ${BIN_TRAFFIC}. Build it first (cargo build --example traffic-send) or use --build." >&2
    exit 1
  fi
  if [[ ! -x "${NS_WRAP}" ]]; then
    echo "[!!] missing ${NS_WRAP}." >&2
    exit 1
  fi

  # pids for cleanup
  host_pid=""
  tcpdump_pid=""
  tmpdir="$(mktemp -d)"
  host_log="${tmpdir}/host.log"
  client_log="${tmpdir}/client.log"
  tcpdump_log="${tmpdir}/tcpdump.log"
  tcpdump_pcap="${tmpdir}/tcpdump.pcap"

  cleanup() {
    # best-effort kill in reverse order
    if [[ -n "${tcpdump_pid}" ]] && kill -0 "${tcpdump_pid}" >/dev/null 2>&1; then
      kill "${tcpdump_pid}" >/dev/null 2>&1 || true
      wait "${tcpdump_pid}" >/dev/null 2>&1 || true
    fi
    if [[ -n "${host_pid}" ]] && kill -0 "${host_pid}" >/dev/null 2>&1; then
      kill "${host_pid}" >/dev/null 2>&1 || true
      wait "${host_pid}" >/dev/null 2>&1 || true
    fi
  }
  trap cleanup EXIT INT TERM

  echo "[..] starting host runtime (Ntx --mode net)"
  # Capture logs even if stdout is block-buffered.
  if have_cmd stdbuf; then
    stdbuf -oL -eL "${BIN_NTX}" --mode net --iface "${IFACE_HOST}" --backend "${BACKEND}" --port "${PORT}" >"${host_log}" 2>&1 &
  else
    "${BIN_NTX}" --mode net --iface "${IFACE_HOST}" --backend "${BACKEND}" --port "${PORT}" >"${host_log}" 2>&1 &
  fi
  host_pid=$!

  # Give the host a moment to open socket and start polling.
  sleep 0.3

  if $DO_TCPDUMP; then
    if have_cmd tcpdump; then
      echo "[..] starting tcpdump on ${IFACE_HOST}"
      # Save a .pcap for offline analysis, and also keep a short text log.
      # Note: tcpdump writes binary pcap; this file can be opened by Wireshark.
      tcpdump -ni "${IFACE_HOST}" -U -w "${tcpdump_pcap}" "arp or (udp and (port ${PORT} or port 40000))" >"${tcpdump_log}" 2>&1 &
      tcpdump_pid=$!
    else
      echo "[!!] --tcpdump requested but tcpdump not found; skipping." >&2
    fi
  fi

  echo "[..] running traffic-send in netns ${NS}"
  rr_args=(--rr)
  arp_args=(--arp)
  if [[ "${BACKEND}" == "afpacket-dgram" || "${BACKEND}" == "cooked" || "${BACKEND}" == "afpacket_dgram" ]]; then
    # cooked backend is L3-oriented; no ARP.
    arp_args=()
  fi
  # Use timeout so that we always end.
  set +e
  timeout "${TIMEOUT_SECS}" "${NS_WRAP}" "${BIN_TRAFFIC}" \
    --iface "${IFACE_NS}" \
    --backend "${BACKEND}" \
    --dst-ips "${HOST_IP}" \
    --src-ip "${NS_IP}" \
    --dst-port "${PORT}" \
    --src-port 40000 \
    "${rr_args[@]}" "${arp_args[@]}" \
    --pps "${PPS}" \
    --count "${COUNT}" \
    --verbose >"${client_log}" 2>&1
  client_rc=$?
  set -e

  # Give the host a moment to flush periodic stats/logs.
  sleep 0.2

  echo
  echo "== summary =="
  echo "client exit code: ${client_rc}"

  # Pull a few useful lines without being too opinionated about output format.
  if [[ -s "${client_log}" ]]; then
    echo "-- client (last 30 lines) --"
    tail -n 30 "${client_log}" || true
  else
    echo "[!!] client log empty"
  fi

  if [[ -s "${host_log}" ]]; then
    echo "-- host (last 30 lines) --"
    tail -n 30 "${host_log}" || true
  else
    echo "[!!] host log empty"
  fi

  if [[ -n "${tcpdump_pid}" && -s "${tcpdump_log}" ]]; then
    echo "-- tcpdump (last 20 lines) --"
    tail -n 20 "${tcpdump_log}" || true
  fi

  if [[ -n "${tcpdump_pid}" ]]; then
    if [[ -s "${tcpdump_pcap}" ]]; then
      echo "tcpdump pcap: ${tcpdump_pcap}"
    else
      echo "[!!] tcpdump pcap not found (expected: ${tcpdump_pcap})"
    fi
  fi

  echo
  echo "logs saved in: ${tmpdir}"

  # Return the client exit code.
  exit "${client_rc}"
fi

cat <<'EOF'
== Build (run as normal user) ==
  cargo build
  cargo build --example userspace-udp-echo
  cargo build --example traffic-send

== Runbook (3 terminals) ==

Terminal A (host runtime + guest component, MVP-0):
  sudo ./target/debug/Ntx --mode net --iface ntx0 --backend tpacketv3 --port 10001

Terminal B (client in netns -> host runtime):
  sudo ./scripts/ntxns1.sh ./target/debug/examples/traffic-send --iface ntx1 --backend tpacketv3 --dst-ips 10.0.0.1 --src-ip 10.0.0.2 --dst-port 10001 --src-port 40000 --rr --arp --pps 50 --count 50

---

Legacy runbook (userspace-udp-echo on both sides):

Terminal A (host echo):
  sudo ./target/debug/examples/userspace-udp-echo --iface ntx0 --backend tpacketv3 --port 10001 --verbose

Terminal B (netns echo):
  sudo ./scripts/ntxns1.sh ./target/debug/examples/userspace-udp-echo --iface ntx1 --backend tpacketv3 --port 10001 --verbose

Terminal C (client host -> netns):
  sudo ./target/debug/examples/traffic-send --iface ntx0 --backend tpacketv3 --dst-ips 10.0.0.2 --src-ip 10.0.0.1 --dst-port 10001 --src-port 40000 --rr --arp --pps 50 --count 50

Optional tcpdump (host):
  sudo tcpdump -ni ntx0 -vv -e 'arp or (udp and (port 10001 or port 40000))'
EOF
