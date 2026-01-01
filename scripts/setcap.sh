#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'EOF'
Usage:
	scripts/setcap.sh [--bin <path>] [--profile debug|release] [--dry-run]

Sets Linux capabilities required for AF_PACKET:
	CAP_NET_RAW + CAP_NET_ADMIN

Defaults:
	- Auto-detect repo root from this script location.
	- Auto-pick binary at:
			<repo>/target/<profile>/Ntx (preferred)
			<repo>/target/<profile>/ntx
	- profile: debug

Examples:
	scripts/setcap.sh
	scripts/setcap.sh --profile release
	scripts/setcap.sh --bin ./target/debug/Ntx
	scripts/setcap.sh --dry-run
EOF
}

die() {
	echo "error: $*" >&2
	exit 1
}

repo_root() {
	local script_dir
	script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
	# scripts/ is one level under repo root.
	(cd "$script_dir/.." && pwd)
}

bin_path=""
profile="debug"
dry_run="0"

while [[ $# -gt 0 ]]; do
	case "$1" in
		-h|--help)
			usage
			exit 0
			;;
		--bin)
			shift
			[[ $# -gt 0 ]] || die "--bin requires a value"
			bin_path="$1"
			shift
			;;
		--profile)
			shift
			[[ $# -gt 0 ]] || die "--profile requires a value"
			profile="$1"
			shift
			;;
		--dry-run)
			dry_run="1"
			shift
			;;
		*)
			die "unknown arg: $1 (try --help)"
			;;
	esac
done

case "$profile" in
	debug|release) ;;
	*) die "invalid --profile: $profile (expected debug|release)" ;;
esac

root="$(repo_root)"

if [[ -z "$bin_path" ]]; then
	if [[ -x "$root/target/$profile/Ntx" ]]; then
		bin_path="$root/target/$profile/Ntx"
	elif [[ -x "$root/target/$profile/ntx" ]]; then
		bin_path="$root/target/$profile/ntx"
	else
		die "could not find executable Ntx/ntx under $root/target/$profile (build it first)"
	fi
else
	# Resolve relative --bin against repo root for convenience.
	if [[ "$bin_path" != /* ]]; then
		bin_path="$root/$bin_path"
	fi
	[[ -x "$bin_path" ]] || die "binary not found or not executable: $bin_path"
fi

cap_str="cap_net_raw,cap_net_admin+ep"

echo "repo: $root"
echo "bin : $bin_path"

if [[ "$dry_run" == "1" ]]; then
	echo "dry-run: sudo setcap $cap_str $bin_path"
	exit 0
fi

command -v sudo >/dev/null 2>&1 || die "sudo not found"
command -v setcap >/dev/null 2>&1 || die "setcap not found (install libcap2-bin)"

sudo setcap "$cap_str" "$bin_path"

if command -v getcap >/dev/null 2>&1; then
	echo "getcap: $(getcap "$bin_path" || true)"
else
	echo "note: getcap not found; skipping verification"
fi

echo "ok"