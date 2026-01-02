#!/usr/bin/env bash
set -euo pipefail

# Create a veth pair and move one end into a container network namespace.
#
# This implements “方案 3”: host has `ntx-server`, container has `ntx-client`, forming a direct veth pair.
#
# Typical flow:
#   docker compose -f .build/docker-compose.yml up -d
#   sudo ./scripts/ntx-veth-pair-to-container.sh attach
#   # host side server binds:   ntx-server
#   # container side ntx binds: ntx-client
#
# Detach/cleanup:
#   sudo ./scripts/ntx-veth-pair-to-container.sh detach
#
# Notes:
# - This is pure L2 plumbing; no IP configuration is applied.
# - If the container restarts, the interface inside it will disappear; re-run attach.
# - IMPORTANT: This only works when containers run on the *same Linux kernel* as the host
#   (native Linux Docker / rootless Docker). If you're using Docker Desktop, containers run
#   inside a LinuxKit VM, and the host cannot access container netns to inject veth.

usage() {
  cat <<EOF
Usage:
  $0 attach|detach [container]

Defaults (can be overridden by env or args):
  container: inferred from docker compose service (default: ntx-backend)
  host iface:      ntx-server
  container iface: ntx-client

Args:
  attach|detach            action
  [container]              container name or id (optional)

Environment overrides:
  COMPOSE_FILE   docker compose file (default: .build/docker-compose.yml)
  SERVICE        compose service name (default: ntx-backend)
  HOST_IFACE     host-side veth name (default: ntx-server)
  CONT_IFACE     container-side veth name (default: ntx-client)

Examples:
  docker compose -f .build/docker-compose.yml up -d
  sudo $0 attach

  # Explicit container name/id:
  sudo $0 attach build-ntx-backend-1

  # Custom iface names:
  sudo HOST_IFACE=ntx0 CONT_IFACE=ntx1 $0 attach
EOF
}

log() { echo "[ntx-veth-pair] $*"; }
die() { echo "error: $*" >&2; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

action="${1:-}"
case "${action}" in
  attach|detach)
    ;;
  -h|--help|help|"")
    usage
    exit 0
    ;;
  *)
    die "invalid action: ${action}"
    ;;
esac

# Root is required for ip-link + netns operations.
if [[ ${EUID} -ne 0 ]]; then
  die "this script must be run as root (use sudo)"
fi

need_cmd ip
need_cmd nsenter
need_cmd docker

COMPOSE_FILE="${COMPOSE_FILE:-.build/docker-compose.yml}"
SERVICE="${SERVICE:-ntx-backend}"
HOST_IFACE="${HOST_IFACE:-ntx-server}"
CONT_IFACE="${CONT_IFACE:-ntx-client}"

container_arg="${2:-}"

DOCKER_ROOT=(docker)
DOCKER_USER=(docker)
if [[ -n "${SUDO_USER:-}" ]]; then
  DOCKER_USER=(sudo -u "${SUDO_USER}" docker)
fi

docker_os_root="$(${DOCKER_ROOT[@]} info -f '{{.OperatingSystem}}' 2>/dev/null || true)"
docker_ctx_root="$(${DOCKER_ROOT[@]} context show 2>/dev/null || true)"
docker_os_user="$(${DOCKER_USER[@]} info -f '{{.OperatingSystem}}' 2>/dev/null || true)"
docker_ctx_user="$(${DOCKER_USER[@]} context show 2>/dev/null || true)"

is_docker_desktop() {
  echo "${1}" | grep -qi 'docker desktop'
}

# IMPORTANT: scheme-3 veth injection only works on native Linux dockerd (same kernel as host).
# If we only see the container in Docker Desktop, we should fail with a clear explanation.
if is_docker_desktop "${docker_os_root}" && is_docker_desktop "${docker_os_user}"; then
  die "Docker Desktop detected (root_ctx=${docker_ctx_root:-?} user_ctx=${docker_ctx_user:-?}). 方案3 requires native Linux dockerd; Docker Desktop runs containers inside a VM so host<->container veth injection is not possible."
fi

resolve_container_id() {
  local c="${container_arg}" id
  if [[ -n "${c}" ]]; then
    echo "${c}"
    return 0
  fi

  # First try: docker compose by service name.
  id="$("${DOCKER_ROOT[@]}" compose -f "${COMPOSE_FILE}" ps -q "${SERVICE}" 2>/dev/null | head -n1 || true)"

  # If root can't see the compose project (common when user uses Docker Desktop context), try the invoking user's docker.
  if [[ -z "${id}" && -n "${SUDO_USER:-}" ]]; then
    id="$("${DOCKER_USER[@]}" compose -f "${COMPOSE_FILE}" ps -q "${SERVICE}" 2>/dev/null | head -n1 || true)"
  fi

  # Fallback: match by running container name (handles names like `ntx-backend-1` or `build-ntx-backend-1`).
  if [[ -z "${id}" ]]; then
    id="$("${DOCKER_ROOT[@]}" ps -q --filter "name=${SERVICE}" 2>/dev/null | head -n1 || true)"
    if [[ -z "${id}" && -n "${SUDO_USER:-}" ]]; then
      id="$("${DOCKER_USER[@]}" ps -q --filter "name=${SERVICE}" 2>/dev/null | head -n1 || true)"
    fi
  fi

  # Fallback: match by compose label (works even if COMPOSE_FILE differs, as long as labels exist).
  if [[ -z "${id}" ]]; then
    id="$("${DOCKER_ROOT[@]}" ps -q --filter "label=com.docker.compose.service=${SERVICE}" 2>/dev/null | head -n1 || true)"
    if [[ -z "${id}" && -n "${SUDO_USER:-}" ]]; then
      id="$("${DOCKER_USER[@]}" ps -q --filter "label=com.docker.compose.service=${SERVICE}" 2>/dev/null | head -n1 || true)"
    fi
  fi

  if [[ -z "${id}" ]]; then
    msg="cannot infer container id.\n\nThis usually happens when:\n- you started compose using a different docker context/daemon than root uses, OR\n- SERVICE is not the compose service name.\n\nTry one of:\n  sudo $0 attach <container-name>\n  sudo SERVICE=ntx-backend $0 attach\n\nDebug info:\n- root docker: context=${docker_ctx_root:-?} os=${docker_os_root:-?}\n- user docker: context=${docker_ctx_user:-?} os=${docker_os_user:-?}\n\nNotes:\n- SERVICE is a compose service name (e.g. 'ntx-backend')\n- container names are often suffixed (e.g. 'ntx-backend-1' or 'build-ntx-backend-1')\n- 方案3 requires the container to run on native Linux dockerd (not Docker Desktop)."
    die "$msg"
  fi
  echo "${id}"
}

container_id="$(resolve_container_id)"

pid="$(${DOCKER_ROOT[@]} inspect -f '{{.State.Pid}}' "${container_id}" 2>/dev/null | tr -d '\r' || true)"
if [[ -z "${pid}" && -n "${SUDO_USER:-}" ]]; then
  pid="$(${DOCKER_USER[@]} inspect -f '{{.State.Pid}}' "${container_id}" 2>/dev/null | tr -d '\r' || true)"
fi
if [[ -z "${pid}" || "${pid}" == "0" ]]; then
  die "failed to resolve container PID for '${container_id}'"
fi

# If the container is running under Docker Desktop (VM), /proc/<pid> won't exist on the host.
if [[ ! -e "/proc/${pid}/ns/net" ]]; then
  # Try to provide a clearer hint if the user docker is Docker Desktop.
  if is_docker_desktop "${docker_os_user}"; then
    die "container PID ${pid} is not in host /proc, which indicates the container is not running on the host kernel (likely Docker Desktop context '${docker_ctx_user}'). 方案3 veth injection requires native dockerd. Switch to native: 'docker context use default' and start compose there."
  fi
  die "container PID ${pid} is not in host /proc; cannot inject veth into container netns. Ensure the container runs on native Linux dockerd."
fi

iface_exists_host() {
  ip link show "$1" >/dev/null 2>&1
}

iface_exists_in_container() {
  nsenter -t "${pid}" -n ip link show "$1" >/dev/null 2>&1
}

netns_name_for_pid() {
  echo "ntx-cont-${pid}"
}

with_named_netns() {
  # Some `ip` builds don't accept `ip link set dev netns <PID>` and only accept netns NAME.
  # Provide NAME by creating /var/run/netns/<NAME> -> /proc/<PID>/ns/net.
  local ns_name
  ns_name="$(netns_name_for_pid)"
  mkdir -p /var/run/netns
  ln -sf "/proc/${pid}/ns/net" "/var/run/netns/${ns_name}"
  "$@" "${ns_name}"
  rm -f "/var/run/netns/${ns_name}" || true
}

case "${action}" in
  attach)
    log "container=${container_id} pid=${pid}"
    log "host_if=${HOST_IFACE} container_if=${CONT_IFACE}"

    # Clean any previous leftovers (idempotent-ish).
    if iface_exists_host "${HOST_IFACE}"; then
      log "host iface '${HOST_IFACE}' already exists; deleting it first"
      ip link del "${HOST_IFACE}" || true
    fi
    if iface_exists_in_container "${CONT_IFACE}"; then
      log "container iface '${CONT_IFACE}' already exists; deleting it first"
      nsenter -t "${pid}" -n ip link del "${CONT_IFACE}" || true
    fi

    # Create veth pair on host.
    ip link add "${HOST_IFACE}" type veth peer name "${CONT_IFACE}"

    # Move container end into container netns.
    with_named_netns ip link set "${CONT_IFACE}" netns

    # Bring up both ends.
    ip link set "${HOST_IFACE}" up
    nsenter -t "${pid}" -n ip link set "${CONT_IFACE}" up

    log "[ok] created veth pair"
    log "- host:      ${HOST_IFACE}"
    log "- container: ${CONT_IFACE}"

    ip -o link show "${HOST_IFACE}" | sed 's/^/[host] /'
    nsenter -t "${pid}" -n ip -o link show "${CONT_IFACE}" | sed 's/^/[cont] /'

    log "next: run server on host with iface=${HOST_IFACE}, and run ntx inside container with nic.iface=${CONT_IFACE}"
    ;;

  detach)
    log "container=${container_id} pid=${pid}"
    log "host_if=${HOST_IFACE} container_if=${CONT_IFACE}"

    if iface_exists_host "${HOST_IFACE}"; then
      ip link del "${HOST_IFACE}" || true
      log "[ok] deleted host iface '${HOST_IFACE}' (veth pair removed)"
      exit 0
    fi

    # If host end is gone but container end remains, try to delete inside container.
    if iface_exists_in_container "${CONT_IFACE}"; then
      nsenter -t "${pid}" -n ip link del "${CONT_IFACE}" || true
      log "[ok] deleted container iface '${CONT_IFACE}'"
      exit 0
    fi

    log "nothing to do (no '${HOST_IFACE}' on host, no '${CONT_IFACE}' in container)"
    ;;
esac
