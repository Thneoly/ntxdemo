#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'EOF'
Usage:
	start.sh [all|backend|frontend]

Defaults:
	- NTX_HOME defaults to /opt/ntx
	- Frontend served from $NTX_HOME/frontend
	- Backend started as: $NTX_HOME/ntx-backend --config $NTX_HOME/config/ntx-backend.yaml

Env vars:
	NTX_HOME         override install dir (default: /opt/ntx)
	FRONTEND_HOST    default: 127.0.0.1
	FRONTEND_PORT    default: 5173
EOF
}

log() {
	echo "[start] $*"
}

die() {
	echo "error: $*" >&2
	exit 1
}

need_cmd() {
	command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

cmd="${1:-all}"
case "$cmd" in
	-h|--help|help)
		usage
		exit 0
		;;
	all|backend|frontend)
		;;
	*)
		die "unknown command: $cmd (try: $0 --help)"
		;;
esac

NTX_HOME="${NTX_HOME:-/opt/ntx}"
FRONTEND_HOST="${FRONTEND_HOST:-127.0.0.1}"
FRONTEND_PORT="${FRONTEND_PORT:-5173}"

backend_bin="$NTX_HOME/ntx-backend"
backend_cfg="$NTX_HOME/config/ntx-backend.yaml"
frontend_dir="$NTX_HOME/frontend"
run_dir="$NTX_HOME/run"

mkdir -p "$run_dir"

backend_pid=""
cleanup() {
	if [[ -n "$backend_pid" ]]; then
		if kill -0 "$backend_pid" >/dev/null 2>&1; then
			log "stopping backend (pid=$backend_pid)"
			kill "$backend_pid" >/dev/null 2>&1 || true
			wait "$backend_pid" >/dev/null 2>&1 || true
		fi
	fi
}
trap cleanup EXIT INT TERM

start_backend_background() {
	[[ -x "$backend_bin" ]] || die "backend binary not found or not executable: $backend_bin"
	[[ -f "$backend_cfg" ]] || die "backend config not found: $backend_cfg"

	log "starting backend (background): $backend_bin --config $backend_cfg"
	cd "$NTX_HOME"
	"$backend_bin" --config "$backend_cfg" \
		>"$run_dir/ntx-backend.log" 2>&1 &
	backend_pid="$!"
	printf '%s' "$backend_pid" >"$run_dir/ntx-backend.pid"
	log "backend started (pid=$backend_pid, log=$run_dir/ntx-backend.log)"
}

start_backend_foreground() {
	[[ -x "$backend_bin" ]] || die "backend binary not found or not executable: $backend_bin"
	[[ -f "$backend_cfg" ]] || die "backend config not found: $backend_cfg"
	need_cmd tee

	log "starting backend (foreground): $backend_bin --config $backend_cfg"
	log "logs: stdout + $run_dir/ntx-backend.log"
	cd "$NTX_HOME"
	# Run in foreground so logs appear in this terminal.
	# Also append to log file for later inspection.
	"$backend_bin" --config "$backend_cfg" 2>&1 | tee -a "$run_dir/ntx-backend.log"
}

start_frontend() {
	[[ -d "$frontend_dir" ]] || die "frontend directory not found: $frontend_dir"
	[[ -f "$frontend_dir/index.html" ]] || die "frontend index.html not found: $frontend_dir/index.html"

	if command -v python3 >/dev/null 2>&1; then
		py=python3
	elif command -v python >/dev/null 2>&1; then
		py=python
	else
		die "python3/python not found; cannot serve frontend"
	fi

	log "serving frontend: http://$FRONTEND_HOST:$FRONTEND_PORT (dir=$frontend_dir)"
	# Keep this in the foreground so Ctrl-C stops everything.
	"$py" -m http.server "$FRONTEND_PORT" --bind "$FRONTEND_HOST" --directory "$frontend_dir"
}

case "$cmd" in
	backend)
		start_backend_foreground
		;;
	frontend)
		start_frontend
		;;
	all)
		start_backend_background
		start_frontend
		;;
esac
