#!/usr/bin/env bash
set -euo pipefail

# Attach/detach the host-side veth (default: ntx0) to the Docker Compose bridge.
#
# Use-case:
# - You created veth pair + netns via: sudo ./scripts/ntx-veth-up.sh
# - You want the netns side (ntxns1:ntx1) to be on the same L2 domain as a container's eth0
#   (the compose bridge), so AF_PACKET traffic can flow between them.
#
# Examples:
#   sudo ./scripts/ntx-veth-attach-compose-bridge.sh attach
#   sudo ./scripts/ntx-veth-attach-compose-bridge.sh detach
#
# Overrides:
#   IFACE=ntx0
#   COMPOSE_FILE=.build/docker-compose.yml
#   SERVICE=ntx-backend
#   BRIDGE=br-xxxxxxxxxxxxxxxx

detect_rootless_docker_host() {
  # If we're running under sudo and the machine uses rootless Docker,
  # the daemon socket is typically at /run/user/<uid>/docker.sock.
  # In that case, `sudo docker ...` will otherwise talk to /var/run/docker.sock
  # and won't see user containers/networks.
  if [[ ${EUID} -eq 0 && -n "${SUDO_UID:-}" && -z "${DOCKER_HOST:-}" ]]; then
    local sock="/run/user/${SUDO_UID}/docker.sock"
    if [[ -S "${sock}" ]]; then
      export DOCKER_HOST="unix://${sock}"
    fi
  fi
}

usage() {
  cat <<EOF
Usage:
  $0 attach|detach

Environment overrides:
  IFACE         host-side veth to attach/detach (default: ntx0)
  COMPOSE_FILE  docker compose file (default: .build/docker-compose.yml)
  SERVICE       compose *service name* used to infer network (default: ntx-backend)
  BRIDGE        linux bridge name (if set, skips auto-detection)
EOF
}

if [[ ${EUID} -ne 0 ]]; then
  echo "This script must be run as root." >&2
  exit 1
fi

action="${1:-}"
case "${action}" in
  attach|detach) ;;
  -h|--help|help|"")
    usage
    exit 0
    ;;
  *)
    echo "invalid action: ${action}" >&2
    usage
    exit 1
    ;;
esac

IFACE="${IFACE:-ntx0}"
COMPOSE_FILE="${COMPOSE_FILE:-.build/docker-compose.yml}"
SERVICE="${SERVICE:-ntx-backend}"

if ! ip link show "${IFACE}" >/dev/null 2>&1; then
  echo "iface '${IFACE}' not found. Did you run?" >&2
  echo "  sudo ./scripts/ntx-veth-up.sh" >&2
  exit 1
fi

bridge="${BRIDGE:-}"
if [[ -z "${bridge}" ]]; then
  if ! command -v docker >/dev/null 2>&1; then
    echo "docker not found; set BRIDGE=... to specify bridge explicitly" >&2
    exit 1
  fi

  # Docker queries are better run as the original (non-root) user:
  # - Works with rootless Docker (root cannot see user containers by default)
  # - Preserves the user's docker context
  DOCKER=(docker)
  if [[ ${EUID} -eq 0 && -n "${SUDO_USER:-}" ]]; then
    DOCKER=(sudo -u "${SUDO_USER}" docker)
  else
    detect_rootless_docker_host
  fi

  container_id=""

  # First try: treat SERVICE as a compose service name.
  services="$("${DOCKER[@]}" compose -f "${COMPOSE_FILE}" config --services 2>/dev/null || true)"
  if echo "${services}" | awk '{print $1}' | grep -qx "${SERVICE}"; then
    container_id="$("${DOCKER[@]}" compose -f "${COMPOSE_FILE}" ps -q "${SERVICE}" 2>/dev/null | head -n1 || true)"
  fi

  # Fallback: if SERVICE isn't a compose service name, treat it as a container name filter.
  if [[ -z "${container_id}" ]]; then
    container_id="$("${DOCKER[@]}" ps -aq --filter "name=${SERVICE}" 2>/dev/null | head -n1 || true)"
  fi

  if [[ -z "${container_id}" ]]; then
    echo "compose service '${SERVICE}' container not found/running; set BRIDGE=... or start compose first" >&2
    echo "  docker compose -f ${COMPOSE_FILE} up -d" >&2
    echo "hint: SERVICE expects a compose service name (e.g. 'ntx-backend'), not a container name (e.g. 'build-ntx-backend-1')" >&2
    if [[ -n "${DOCKER_HOST:-}" ]]; then
      echo "hint: using DOCKER_HOST=${DOCKER_HOST}" >&2
    fi
    exit 1
  fi

  network_name="$("${DOCKER[@]}" inspect -f '{{range $k,$v := .NetworkSettings.Networks}}{{$k}}{{"\n"}}{{end}}' "${container_id}" | head -n1 | tr -d '\r' || true)"
  if [[ -z "${network_name}" ]]; then
    echo "failed to detect docker network name for container: ${container_id}" >&2
    exit 1
  fi

  bridge="$("${DOCKER[@]}" network inspect -f '{{ index .Options "com.docker.network.bridge.name" }}' "${network_name}" 2>/dev/null | tr -d '\r' || true)"
  if [[ -z "${bridge}" ]]; then
    # Fallback for many Docker setups: linux bridge name is derived from network ID.
    # The interface is typically: br-<first 12 chars of network id>
    net_id="$("${DOCKER[@]}" network inspect -f '{{.Id}}' "${network_name}" 2>/dev/null | tr -d '\r' || true)"
    if [[ -n "${net_id}" ]]; then
      bridge="br-${net_id:0:12}"
    fi
  fi

  if [[ -z "${bridge}" ]]; then
    echo "failed to detect linux bridge name for docker network '${network_name}'" >&2
    echo "hint: set BRIDGE=br-... to specify bridge explicitly" >&2
    exit 1
  fi
fi

if ! ip link show "${bridge}" >/dev/null 2>&1; then
  # Fallback: if there's exactly one br-* interface on the host, use it.
  # (Some Docker setups don't expose com.docker.network.bridge.name and don't map br- to network id.)
  mapfile -t br_candidates < <(ip -o link show | awk -F': ' '$2 ~ /^br-[0-9a-f]+$/ {print $2}' | sort -u)
  if [[ ${#br_candidates[@]} -eq 1 ]]; then
    bridge="${br_candidates[0]}"
  else
    echo "bridge '${bridge}' not found (not a linux link)." >&2
    echo "hint: your Docker may be running in a mode without a linux bridge (e.g. rootless / Docker Desktop user-mode networking)." >&2
    echo "hint: if you *do* have a host bridge, set BRIDGE=br-... explicitly (you can find it via: ip -o link show | awk -F\": \" '$2 ~ /^br-/' )" >&2
    exit 1
  fi
fi

# Keep the veth up (attaching to a master does not automatically bring it up).
ip link set "${IFACE}" up || true

if [[ "${action}" == "attach" ]]; then
  ip link set "${IFACE}" master "${bridge}"
  echo "[ok] attached ${IFACE} -> ${bridge}"
else
  ip link set "${IFACE}" nomaster
  echo "[ok] detached ${IFACE} (nomaster)"
fi

# Show a quick summary.
if ip -o link show "${IFACE}" | grep -q 'master'; then
  ip -o link show "${IFACE}" | sed 's/^/[info] /'
else
  ip -o link show "${IFACE}" | sed 's/^/[info] /'
fi
