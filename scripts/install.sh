#!/usr/bin/env bash
set -euo pipefail

# Local install helper.
#
# Order:
#  1) Run release packaging -> dist/
#  2) Move/rename binaries + unpack config into /tmp/ntx
#  3) cargo build --example ntx-echo-server -> move to /tmp/ntx
#  4) Run veth setup
#  5) setcap for ntx
#  6) setcap for ntx-echo-server

usage() {
	cat <<'EOF'
Usage:
	scripts/install.sh [install|push|pull]

Commands:
	install  Build release assets and install binaries/config/scripts (default)
	push     Build component WASMs and push to Harbor via ORAS
	pull     Pull component WASMs from Harbor into /opt/ntx/component/wac/deps/component (or NTX_INSTALL_DIR)

Env vars (push/pull):
	HARBOR_REGISTRY      default: 192.168.31.138
	WASM_TAG             default: v0.0.1
	HARBOR_EVENTBUS_REF  override full ref (e.g. 192.168.31.138/ntx/eventbus:v0.0.1)
	HARBOR_SCHEDULER_REF override full ref (e.g. 192.168.31.138/ntx/scheduler:v0.0.1)
	WASM_ARTIFACT_TYPE   default: application/vnd.ntx.wasm.v1

Optional ORAS auth/tls:
	HARBOR_USER, HARBOR_PASS, HARBOR_CA_FILE, ORAS_INSECURE=1, ORAS_PLAIN_HTTP=1

.env:
	This script loads repo-root .env if present; otherwise it falls back to .env.example.
	Already-exported environment variables take precedence.

Install location:
	NTX_INSTALL_DIR      default: /opt/ntx

Frontend:
	SKIP_FRONTEND_BUILD=1  skip building frontend/demo-workflow
EOF
}

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

load_dotenv() {
	local env_file="$ROOT_DIR/.env"
	if [[ ! -f "$env_file" ]]; then
		env_file="$ROOT_DIR/.env.example"
	fi
	[[ -f "$env_file" ]] || return 0

	# Minimal .env loader:
	# - supports KEY=VALUE (no export keyword)
	# - ignores blank lines and comments
	# - strips surrounding single/double quotes
	# - does NOT override already-set env vars
	while IFS= read -r line || [[ -n "$line" ]]; do
		# Trim leading whitespace
		line="${line#${line%%[![:space:]]*}}"
		[[ -z "$line" ]] && continue
		[[ "$line" == \#* ]] && continue
		[[ "$line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]] || continue

		key="${line%%=*}"
		val="${line#*=}"
		# Trim surrounding whitespace
		val="${val#${val%%[![:space:]]*}}"
		val="${val%${val##*[![:space:]]}}"

		# Skip if already set in environment
		if [[ -n "${!key+x}" ]]; then
			continue
		fi

		# Strip matching quotes
		if [[ "$val" =~ ^".*"$ ]]; then
			val="${val#\"}"
			val="${val%\"}"
		elif [[ "$val" =~ ^'.*'$ ]]; then
			val="${val#\'}"
			val="${val%\'}"
		fi

		export "$key=$val"
	done < "$env_file"
}

load_dotenv

DIST_DIR="$ROOT_DIR/dist"
INSTALL_DIR="${NTX_INSTALL_DIR:-/opt/ntx}"
INSTALL_CONFIG_DIR="$INSTALL_DIR/config"
INSTALL_SCRIPT_DIR="$INSTALL_DIR/script"
INSTALL_ORAS_SCRIPT_DIR="$INSTALL_SCRIPT_DIR/oras"

log() {
	echo "[install] $*"
}

die() {
	echo "error: $*" >&2
	exit 1
}

need_cmd() {
	command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

need_cmd bash
need_cmd awk
need_cmd cargo
need_cmd find
need_cmd git
need_cmd install
need_cmd rustc
need_cmd sha256sum
need_cmd sort
need_cmd tar
need_cmd unzip
need_cmd zip
need_cmd sudo
need_cmd mktemp
need_cmd cp

patch_installed_backend_config() {
	local cfg="$INSTALL_CONFIG_DIR/ntx-backend.yaml"
	[[ -f "$cfg" ]] || return 0

	# Always use the installed ntx binary.
	local desired_ntx_bin="$__INSTALL_DIR_FOR_PATCH/ntx"
	local desired_wac_compose_bin="$__INSTALL_DIR_FOR_PATCH/ntx-wac-compose"
	local desired_wac_compose_cwd="$__INSTALL_DIR_FOR_PATCH"

	# Harbor settings should come from .env when provided.
	local desired_ca_file="${HARBOR_CA_FILE:-}"
	local desired_user="${HARBOR_USER:-}"
	local desired_pass="${HARBOR_PASS:-}"

	local tmp
	tmp="$(mktemp)"

	awk \
		-v desired_ntx_bin="$desired_ntx_bin" \
		-v desired_wac_compose_bin="$desired_wac_compose_bin" \
		-v desired_wac_compose_cwd="$desired_wac_compose_cwd" \
		-v desired_ca_file="$desired_ca_file" \
		-v desired_user="$desired_user" \
		-v desired_pass="$desired_pass" \
		'
		BEGIN {
			header_mode = 1
			header_n = 0
			in_harbor = 0
			harbor_seen = 0
			ntx_bin_written = 0
		}
		function flush_header_and_fixed_top_level() {
			for (i = 1; i <= header_n; i++) print header[i]
			print "ntx_bin: \"" desired_ntx_bin "\""
			print "wac_compose_bin: \"" desired_wac_compose_bin "\""
			print "wac_compose_cwd: \"" desired_wac_compose_cwd "\""
			ntx_bin_written = 1
		}
		function emit_harbor_block() {
			print "harbor:"
			if (desired_ca_file != "") print "  ca_file: " desired_ca_file
			if (desired_user != "") print "  user: " desired_user
			if (desired_pass != "") print "  pass: " desired_pass
		}
		# Header (comments/blanks) at top
		header_mode == 1 {
			if ($0 ~ /^[[:space:]]*#/ || $0 ~ /^[[:space:]]*$/) {
				header[++header_n] = $0
				next
			}
			flush_header_and_fixed_top_level()
			header_mode = 0
			# Continue processing this non-header line
		}

		# Drop any existing top-level ntx_bin (we rewrite it once near top)
		/^[[:space:]]*ntx_bin:[[:space:]]*/ { next }
		/^[[:space:]]*wac_compose_bin:[[:space:]]*/ { next }
		/^[[:space:]]*wac_compose_cwd:[[:space:]]*/ { next }

		# Detect harbor section start (top-level key)
		/^[[:space:]]*harbor:[[:space:]]*$/ {
			harbor_seen = 1
			in_harbor = 1
			emit_harbor_block()
			next
		}

		# While inside harbor block, skip keys we manage.
		in_harbor == 1 {
			# If we hit a new top-level key, exit harbor block.
			if ($0 ~ /^[^[:space:]]/ && $0 !~ /^harbor:[[:space:]]*$/) {
				in_harbor = 0
				# fallthrough to normal printing
			} else {
				if ($0 ~ /^[[:space:]]+ca_file:[[:space:]]*/ || $0 ~ /^[[:space:]]+user:[[:space:]]*/ || $0 ~ /^[[:space:]]+pass:[[:space:]]*/) {
					next
				}
				# For any other indented lines under harbor (comments/other keys), drop them
				# to avoid keeping stale/duplicated content.
				next
			}
		}

		{ print }

		END {
			if (header_mode == 1) {
				flush_header_and_fixed_top_level()
			}
			if (harbor_seen == 0) {
				emit_harbor_block()
			}
		}
		' "$cfg" >"$tmp"

	mv "$tmp" "$cfg"
}

oras_login_if_needed() {
	local -a login_args=()
	local registry="${HARBOR_REGISTRY:-}"
	if [[ -z "$registry" ]]; then
		# Best effort derive from provided refs.
		if [[ -n "${HARBOR_EVENTBUS_REF:-}" ]]; then
			registry="${HARBOR_EVENTBUS_REF%%/*}"
		elif [[ -n "${HARBOR_SCHEDULER_REF:-}" ]]; then
			registry="${HARBOR_SCHEDULER_REF%%/*}"
		fi
	fi

	if [[ -n "${HARBOR_CA_FILE:-}" ]]; then
		login_args+=(--ca-file "$HARBOR_CA_FILE")
	fi
	if [[ "${ORAS_INSECURE:-}" == "1" ]]; then
		login_args+=(--insecure)
	fi
	if [[ "${ORAS_PLAIN_HTTP:-}" == "1" ]]; then
		login_args+=(--plain-http)
	fi

	# Login is optional: if you already logged in (oras credential store), you can omit HARBOR_USER/PASS.
	if [[ -n "${HARBOR_USER:-}" ]]; then
		if [[ -n "${HARBOR_PASS:-}" ]]; then
			printf '%s' "$HARBOR_PASS" | oras login "${login_args[@]}" -u "$HARBOR_USER" --password-stdin "$registry"
		else
			oras login "${login_args[@]}" -u "$HARBOR_USER" "$registry"
		fi
	fi
}

oras_build_args() {
	# Populates a named array variable with common oras args.
	# Usage: local -a args=(); oras_build_args args
	local __var_name="$1"
	# shellcheck disable=SC2178
	local -n __out="$__var_name"

	if [[ -n "${HARBOR_CA_FILE:-}" ]]; then
		__out+=(--ca-file "$HARBOR_CA_FILE")
	fi
	if [[ "${ORAS_INSECURE:-}" == "1" ]]; then
		__out+=(--insecure)
	fi
	if [[ "${ORAS_PLAIN_HTTP:-}" == "1" ]]; then
		__out+=(--plain-http)
	fi
}

mount_options_for_path() {
	local path="$1"
	if command -v findmnt >/dev/null 2>&1; then
		findmnt -no OPTIONS -T "$path" 2>/dev/null || true
		return 0
	fi

	# Fallback: best-effort parse from /proc/mounts (match longest mount point).
	awk -v p="$path" '
		BEGIN { best_len = -1; best_opts = "" }
		{
			mp = $2
			gsub(/\\040/, " ", mp)
			opts = $4
			# Match exact or prefix.
			if (p == mp || index(p, mp "/") == 1) {
				if (length(mp) > best_len) { best_len = length(mp); best_opts = opts }
			}
		}
		END { print best_opts }
	' /proc/mounts 2>/dev/null || true
}

ensure_install_dir_supports_caps() {
	local opts
	local check_path="$INSTALL_DIR"
	if [[ ! -e "$check_path" ]]; then
		check_path="$(dirname "$check_path")"
	fi
	opts="$(mount_options_for_path "$check_path")"
	if [[ "$opts" == *nosuid* ]]; then
		die "INSTALL_DIR=$INSTALL_DIR is on a nosuid mount ($opts); Linux file capabilities (setcap) will NOT take effect. Use NTX_INSTALL_DIR=/opt/ntx, /var/tmp/ntx, or another non-nosuid path."
	fi
}

ensure_install_dir_supports_caps

preflight_sudo() {
	# Avoid hanging in non-interactive contexts (e.g., CI or tool execution).
	if sudo -n true >/dev/null 2>&1; then
		return 0
	fi

	if [[ ! -t 0 ]]; then
		die "sudo is required but no TTY is available; run this script in an interactive terminal"
	fi

	log "sudo required; you may be prompted for your password"
	sudo -v

	# Keep sudo ticket alive while the script runs.
	(
		while true; do
			sudo -n true >/dev/null 2>&1 || exit 0
			sleep 30
		done
	) &
	SUDO_KEEPALIVE_PID=$!
	trap 'kill "$SUDO_KEEPALIVE_PID" >/dev/null 2>&1 || true' EXIT
}

ensure_install_dir_writable() {
	# Ensure INSTALL_DIR exists and is writable by the current user.
	# We want runtime config/scripts to be readable without root.
	if [[ -d "$INSTALL_DIR" && -w "$INSTALL_DIR" ]]; then
		return 0
	fi

	preflight_sudo
	log "Preparing $INSTALL_DIR (sudo mkdir/chown)"
	sudo mkdir -p "$INSTALL_DIR"
	sudo chown -R "$(id -u):$(id -g)" "$INSTALL_DIR"
}

fix_install_permissions() {
	# Ensure config is readable by non-root, scripts executable, etc.
	if [[ -d "$INSTALL_CONFIG_DIR" ]]; then
		find "$INSTALL_CONFIG_DIR" -type d -exec chmod 755 {} +
		find "$INSTALL_CONFIG_DIR" -type f -exec chmod 644 {} +
	fi
	if [[ -d "$INSTALL_SCRIPT_DIR" ]]; then
		find "$INSTALL_SCRIPT_DIR" -type d -exec chmod 755 {} +
		find "$INSTALL_SCRIPT_DIR" -type f -exec chmod 755 {} +
	fi
}

wipe_install_dir() {
	# Clear INSTALL_DIR contents to avoid stale artifacts from previous runs.
	# Safety guards to avoid accidental rm -rf on the wrong path.
	[[ -n "${INSTALL_DIR:-}" ]] || die "INSTALL_DIR is empty"
	[[ "$INSTALL_DIR" != "/" ]] || die "refusing to wipe INSTALL_DIR=/"
	[[ "$INSTALL_DIR" == */ntx ]] || die "refusing to wipe suspicious INSTALL_DIR=$INSTALL_DIR (expected it to end with /ntx)"

	log "Wiping install dir contents under $INSTALL_DIR"
	# Remove both normal and dotfiles (but keep . and ..).
	shopt -s dotglob nullglob
	rm -rf "$INSTALL_DIR"/*
	shopt -u dotglob nullglob
}

detect_harbor_ca_file() {
	# Priority:
	#  1) Explicit env var
	#  2) Installed config under $INSTALL_CONFIG_DIR/ntx-backend.yaml
	#  3) Repo default config under crates/ntx-backend/conf/ntx-backend.yaml
	if [[ -n "${HARBOR_CA_FILE:-}" ]]; then
		echo "$HARBOR_CA_FILE"
		return 0
	fi

	local cfg=""
	if [[ -f "$INSTALL_CONFIG_DIR/ntx-backend.yaml" ]]; then
		cfg="$INSTALL_CONFIG_DIR/ntx-backend.yaml"
	elif [[ -f "$ROOT_DIR/crates/ntx-backend/conf/ntx-backend.yaml" ]]; then
		cfg="$ROOT_DIR/crates/ntx-backend/conf/ntx-backend.yaml"
	fi
	[[ -n "$cfg" ]] || return 0

	# Extract: harbor:
	#   ca_file: /path/to/ca.crt
	awk '
		$1 == "harbor:" { in=1; next }
		in && $1 == "ca_file:" { print $2; exit }
		in && $1 ~ /^[^#].*:/ && $1 != "ca_file:" { exit }
	' "$cfg" 2>/dev/null || true
}

build_frontend() {
	if [[ "${SKIP_FRONTEND_BUILD:-}" == "1" ]]; then
		log "frontend: SKIP_FRONTEND_BUILD=1, skipping"
		return 0
	fi

	local fe_dir="$ROOT_DIR/frontend/demo-workflow"
	[[ -f "$fe_dir/package.json" ]] || die "frontend project not found at $fe_dir (missing package.json)"
	need_cmd npm

	log "frontend: building (npm ci && npm run build)"
	(
		cd "$fe_dir"
		if [[ -f package-lock.json ]]; then
			npm ci
		else
			npm install
		fi
		npm run build
	)

	local fe_dist="$fe_dir/dist"
	[[ -d "$fe_dist" ]] || die "frontend build did not produce dist/ at $fe_dist"

	ensure_install_dir_writable
	local out_dir="$INSTALL_DIR/frontend"
	rm -rf "$out_dir"
	mkdir -p "$out_dir"
	cp -a "$fe_dist/." "$out_dir/"

	# Make files readable to non-root.
	find "$out_dir" -type d -exec chmod 755 {} +
	find "$out_dir" -type f -exec chmod 644 {} +

	log "frontend: installed to $out_dir"
}

do_install() {
	log "Step 1/5: build release assets (dist/)"
	chmod +x "$ROOT_DIR/scripts/release/build-release-assets.sh"
	"$ROOT_DIR/scripts/release/build-release-assets.sh"

	log "Preparing install dirs under $INSTALL_DIR"
	ensure_install_dir_writable
	wipe_install_dir
	mkdir -p "$INSTALL_CONFIG_DIR" "$INSTALL_SCRIPT_DIR" "$INSTALL_ORAS_SCRIPT_DIR"

	[[ -d "$DIST_DIR" ]] || die "missing dist/ (run build-release-assets.sh first)"

	# Find release binaries in dist.
	mapfile -t root_bins < <(find "$DIST_DIR" -maxdepth 1 -type f -name 'ntx-*' \
		! -name 'ntx-backend-*' \
		! -name 'ntx-wit-*' \
		! -name '*.tar.gz' \
		! -name '*.zip' \
		! -name 'SHA256SUMS.txt' \
		| sort)
	mapfile -t backend_bins < <(find "$DIST_DIR" -maxdepth 1 -type f -name 'ntx-backend-*' \
		! -name '*.tar.gz' \
		! -name '*.zip' \
		| sort)

	(( ${#root_bins[@]} == 1 )) || die "expected exactly 1 ntx-<target_triple> in dist/, found ${#root_bins[@]}"
	(( ${#backend_bins[@]} == 1 )) || die "expected exactly 1 ntx-backend-<target_triple> in dist/, found ${#backend_bins[@]}"

	root_bin="${root_bins[0]}"
	backend_bin="${backend_bins[0]}"

	config_zip="$DIST_DIR/config.zip"
	[[ -f "$config_zip" ]] || die "missing dist/config.zip (run build-release-assets.sh first)"

	log "Step 1: install binaries + config into $INSTALL_DIR"
	rm -f "$INSTALL_DIR/ntx" "$INSTALL_DIR/ntx-backend"

	# Copy (do not move) so dist/ remains intact for debugging and repeat installs.
	install -m 755 "$root_bin" "$INSTALL_DIR/ntx"
	install -m 755 "$backend_bin" "$INSTALL_DIR/ntx-backend"

	rm -rf "$INSTALL_CONFIG_DIR"
	mkdir -p "$INSTALL_CONFIG_DIR"
	unzip -q "$config_zip" -d "$INSTALL_CONFIG_DIR"

	# Let config reflect the installation location and .env Harbor settings.
	# (Used by /opt/ntx/start.sh and by backend itself.)
	__INSTALL_DIR_FOR_PATCH="$INSTALL_DIR"
	patch_installed_backend_config

	log "Installing helper scripts into $INSTALL_SCRIPT_DIR"
	for s in ntx-veth-up.sh ntx-veth-down.sh setcap.sh; do
		[[ -f "$ROOT_DIR/scripts/$s" ]] || die "missing script: scripts/$s"
		install -m 755 "$ROOT_DIR/scripts/$s" "$INSTALL_SCRIPT_DIR/$s"
	done

	log "Installing start script into $INSTALL_DIR"
	[[ -f "$ROOT_DIR/scripts/start.sh" ]] || die "missing script: scripts/start.sh"
	install -m 755 "$ROOT_DIR/scripts/start.sh" "$INSTALL_DIR/start.sh"

	log "Installing ORAS helper scripts into $INSTALL_ORAS_SCRIPT_DIR"
	for s in push.sh pull.sh; do
		[[ -f "$ROOT_DIR/scripts/oras/$s" ]] || die "missing script: scripts/oras/$s"
		install -m 755 "$ROOT_DIR/scripts/oras/$s" "$INSTALL_ORAS_SCRIPT_DIR/$s"
	done

	log "Installing WAC composition assets into $INSTALL_DIR/component/wac (no *.wasm)"
	mkdir -p "$INSTALL_DIR/component/wac/deps/component"
	[[ -f "$ROOT_DIR/component/wac/scheduler-composition.wac" ]] || die "missing $ROOT_DIR/component/wac/scheduler-composition.wac"
	install -m 644 "$ROOT_DIR/component/wac/scheduler-composition.wac" "$INSTALL_DIR/component/wac/scheduler-composition.wac"
	# Ensure repo-shipped wasm artifacts are not present; we want the ones pulled via ORAS.
	find "$INSTALL_DIR/component/wac" -type f -name '*.wasm' -delete || true

	log "Step 2: build helper binary ntx-wac-compose (release)"
	(
		cd "$ROOT_DIR"
		cargo build --release -p ntx-wac-compose
	)
	wac_compose_bin="$ROOT_DIR/target/release/ntx-wac-compose"
	[[ -x "$wac_compose_bin" ]] || die "expected ntx-wac-compose binary at $wac_compose_bin not found"
	install -m 755 "$wac_compose_bin" "$INSTALL_DIR/ntx-wac-compose"

	log "Step 3: build example ntx-echo-server (debug)"
	(
		cd "$ROOT_DIR"
		cargo build --example ntx-echo-server
	)

	example_bin="$ROOT_DIR/target/debug/examples/ntx-echo-server"
	[[ -x "$example_bin" ]] || die "expected example binary at $example_bin not found"

	rm -f "$INSTALL_DIR/ntx-echo-server"
	install -m 755 "$example_bin" "$INSTALL_DIR/ntx-echo-server"

	# Let config reflect the installation location and .env Harbor settings.
	# (Used by /opt/ntx/start.sh and by backend itself.)
	__INSTALL_DIR_FOR_PATCH="$INSTALL_DIR"
	patch_installed_backend_config

	log "Step 4: set up veth/netns (requires sudo)"
	preflight_sudo
	sudo "$INSTALL_SCRIPT_DIR/ntx-veth-up.sh"

	log "Step 5: setcap for ntx"
	"$INSTALL_SCRIPT_DIR/setcap.sh" --bin "$INSTALL_DIR/ntx"

	log "Step 6: setcap for ntx-echo-server"
	"$INSTALL_SCRIPT_DIR/setcap.sh" --bin "$INSTALL_DIR/ntx-echo-server"

	fix_install_permissions

	log "Step 7: push WASMs to Harbor"
	do_push

	log "Step 8: pull WASMs into $INSTALL_DIR/component/wac/deps/component"
	do_pull

	log "Step 9: build frontend and copy into $INSTALL_DIR"
	build_frontend

	log "Done"
	log "- binaries: $INSTALL_DIR/{ntx,ntx-backend,ntx-echo-server}"
	log "- config  : $INSTALL_CONFIG_DIR"
	log "- scripts : $INSTALL_SCRIPT_DIR"
	log "- start   : $INSTALL_DIR/start.sh"
	log "Next (backend):  cd $INSTALL_DIR && ./start.sh backend"
	log "Next (frontend): cd $INSTALL_DIR && ./start.sh frontend"
}

do_push() {
	need_cmd oras
	need_cmd wac
	need_cmd wasm-tools

	local registry="${HARBOR_REGISTRY:-192.168.31.138}"
	local tag="${WASM_TAG:-v0.0.1}"
	local eventbus_ref="${HARBOR_EVENTBUS_REF:-$registry/ntx/eventbus:$tag}"
	local scheduler_ref="${HARBOR_SCHEDULER_REF:-$registry/ntx/scheduler:$tag}"
	local artifact_type="${WASM_ARTIFACT_TYPE:-application/vnd.ntx.wasm.v1}"

	log "push: building component WASMs via component/build.sh"
	chmod +x "$ROOT_DIR/component/build.sh"
	(
		cd "$ROOT_DIR/component"
		./build.sh
	)

	local wasm_dir="$ROOT_DIR/target/wasm32-wasip2/debug"
	local eventbus_wasm="$wasm_dir/eventbus.wasm"
	local scheduler_wasm="$wasm_dir/scheduler.wasm"
	[[ -f "$eventbus_wasm" ]] || die "missing $eventbus_wasm (build.sh did not produce it)"
	[[ -f "$scheduler_wasm" ]] || die "missing $scheduler_wasm (build.sh did not produce it)"

	# Require installed oras helper scripts (no repo fallback).
	local push_script="$INSTALL_ORAS_SCRIPT_DIR/push.sh"
	[[ -x "$push_script" ]] || die "missing installed ORAS script: $push_script (run: scripts/install.sh install)"

	local tmp_out_dir="$ROOT_DIR/target/oras-tmp"
	mkdir -p "$tmp_out_dir"

	local ca_file=""
	ca_file="$(detect_harbor_ca_file)"
	if [[ -n "$ca_file" && ! -f "$ca_file" ]]; then
		log "warning: HARBOR_CA_FILE=$ca_file does not exist"
		ca_file=""
	fi
	if [[ -z "$ca_file" && "${ORAS_INSECURE:-}" != "1" && "${ORAS_PLAIN_HTTP:-}" != "1" ]]; then
		log "note: Harbor TLS may be self-signed; set HARBOR_CA_FILE=/path/to/ca.crt or ORAS_INSECURE=1"
	fi

	log "push: $eventbus_ref"
	HARBOR_REGISTRY="$registry" \
	HARBOR_REF="$eventbus_ref" \
	HARBOR_CA_FILE="$ca_file" \
	ARTIFACT_TYPE="$artifact_type" \
	OUTPUT_DIR="$tmp_out_dir" \
	PUSH_WASM_ONLY=1 \
	SKIP_BUILD=1 \
	WASM_PATH="$eventbus_wasm" \
	"$push_script"

	log "push: $scheduler_ref"
	HARBOR_REGISTRY="$registry" \
	HARBOR_REF="$scheduler_ref" \
	HARBOR_CA_FILE="$ca_file" \
	ARTIFACT_TYPE="$artifact_type" \
	OUTPUT_DIR="$tmp_out_dir" \
	PUSH_WASM_ONLY=1 \
	SKIP_BUILD=1 \
	WASM_PATH="$scheduler_wasm" \
	"$push_script"

	log "push: done"
}

do_pull() {
	need_cmd oras

	local registry="${HARBOR_REGISTRY:-192.168.31.138}"
	local tag="${WASM_TAG:-v0.0.1}"
	local eventbus_ref="${HARBOR_EVENTBUS_REF:-$registry/ntx/eventbus:$tag}"
	local scheduler_ref="${HARBOR_SCHEDULER_REF:-$registry/ntx/scheduler:$tag}"

	local wasm_out_dir="$INSTALL_DIR/component/wac/deps/component"
	log "pull: preparing $wasm_out_dir"
	ensure_install_dir_writable
	mkdir -p "$wasm_out_dir"

	local pull_script="$INSTALL_ORAS_SCRIPT_DIR/pull.sh"
	[[ -x "$pull_script" ]] || die "missing installed ORAS script: $pull_script (run: scripts/install.sh install)"

	local ca_file=""
	ca_file="$(detect_harbor_ca_file)"
	if [[ -n "$ca_file" && ! -f "$ca_file" ]]; then
		log "warning: HARBOR_CA_FILE=$ca_file does not exist"
		ca_file=""
	fi
	if [[ -z "$ca_file" && "${ORAS_INSECURE:-}" != "1" && "${ORAS_PLAIN_HTTP:-}" != "1" ]]; then
		log "note: Harbor TLS may be self-signed; set HARBOR_CA_FILE=/path/to/ca.crt or ORAS_INSECURE=1"
	fi

	log "pull: $eventbus_ref -> $wasm_out_dir"
	HARBOR_REGISTRY="$registry" \
	HARBOR_REF="$eventbus_ref" \
	HARBOR_CA_FILE="$ca_file" \
	OUTPUT_DIR="$wasm_out_dir" \
	"$pull_script"

	log "pull: $scheduler_ref -> $wasm_out_dir"
	HARBOR_REGISTRY="$registry" \
	HARBOR_REF="$scheduler_ref" \
	HARBOR_CA_FILE="$ca_file" \
	OUTPUT_DIR="$wasm_out_dir" \
	"$pull_script"

	log "pull: done"
	log "- files: $wasm_out_dir/{eventbus.wasm,scheduler.wasm}"
}

cmd="${1:-install}"
case "$cmd" in
	-h|--help|help)
		usage
		exit 0
		;;
	install)
		do_install
		;;
	push)
		do_push
		;;
	pull)
		do_pull
		;;
	*)
		die "unknown command: $cmd (try: scripts/install.sh --help)"
		;;
esac
