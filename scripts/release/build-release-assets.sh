#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

DIST_DIR="$ROOT_DIR/dist"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

TARGET_TRIPLE="$(rustc -vV | awk -F': ' '/^host: /{print $2}')"
echo "TARGET_TRIPLE=$TARGET_TRIPLE"

echo "==> Building binaries (release)"
# Root package builds a binary named after the package (currently 'ntx').
cargo build --release -p ntx
cargo build --release -p ntx-backend

ROOT_BIN_SRC="$ROOT_DIR/target/release/ntx"
BACKEND_BIN_SRC="$ROOT_DIR/target/release/ntx-backend"

if [[ ! -f "$ROOT_BIN_SRC" ]]; then
  echo "Expected root binary at $ROOT_BIN_SRC not found" >&2
  exit 1
fi
if [[ ! -f "$BACKEND_BIN_SRC" ]]; then
  echo "Expected backend binary at $BACKEND_BIN_SRC not found" >&2
  exit 1
fi

cp "$ROOT_BIN_SRC" "$DIST_DIR/ntx-${TARGET_TRIPLE}"
cp "$BACKEND_BIN_SRC" "$DIST_DIR/ntx-backend-${TARGET_TRIPLE}"

chmod +x "$DIST_DIR/ntx-${TARGET_TRIPLE}" "$DIST_DIR/ntx-backend-${TARGET_TRIPLE}"

echo "==> Packaging instance config"
# Flatten: app.yaml, config.yaml, resource/ at archive root.
(
  cd "$ROOT_DIR/config"
  zip -r "$DIST_DIR/config.zip" \
    app.yaml \
    config.yaml \
    resource \
    >/dev/null
)

# Also ship backend config alongside instance config.
# Put it at the archive root (no extra directories) so it extracts under /tmp/ntx/config.
BACKEND_CONF_SRC="$ROOT_DIR/crates/ntx-backend/conf/ntx-backend.yaml"
if [[ -f "$BACKEND_CONF_SRC" ]]; then
  zip -j "$DIST_DIR/config.zip" "$BACKEND_CONF_SRC" >/dev/null
else
  echo "warning: backend config not found at $BACKEND_CONF_SRC" >&2
fi

echo "==> Packaging WIT bundles"

# Each WIT package folder becomes its own tar.gz.
# Structure mirrors GitHub archive tarballs like:
#   wasi-sockets-0.2.6/wit/...
# We create:
#   ntx-<version>/wit/<folder>/...
VERSION_TAG="${GITHUB_REF_NAME:-}"

# Prefer an exact tag on HEAD when available.
if [[ -z "$VERSION_TAG" ]]; then
  VERSION_TAG="$(git describe --tags --exact-match 2>/dev/null || true)"
fi

# For local packaging (no tag), fall back to the root crate version.
if [[ -z "$VERSION_TAG" ]]; then
  MANIFEST_VERSION="$(awk -F'"' '/^version = "[^"]+"/{print $2; exit}' "$ROOT_DIR/Cargo.toml" || true)"
  if [[ -n "$MANIFEST_VERSION" ]]; then
    VERSION_TAG="v${MANIFEST_VERSION}"
  fi
fi

# Last resort: something stable-ish.
if [[ -z "$VERSION_TAG" ]]; then
  VERSION_TAG="$(git describe --tags --always 2>/dev/null || echo v0.0.0)"
fi

VERSION="${VERSION_TAG#v}"

TMP_WIT_ROOT="$DIST_DIR/_wit_pkg"
rm -rf "$TMP_WIT_ROOT"
mkdir -p "$TMP_WIT_ROOT"

# Only publish selected WIT packages.
WIT_ALLOWLIST=(
  "actions-executor"
  "core-types"
  "eventbus"
)

for src_dir in "$ROOT_DIR/component/wit"/*; do
  [[ -d "$src_dir" ]] || continue
  folder="$(basename "$src_dir")"

  allowed=false
  for allowed_folder in "${WIT_ALLOWLIST[@]}"; do
    if [[ "$folder" == "$allowed_folder" ]]; then
      allowed=true
      break
    fi
  done
  if [[ "$allowed" != "true" ]]; then
    continue
  fi

  # Match GitHub archive behavior: the top-level directory inside the tarball
  # equals the archive base name.
  ARCHIVE_ROOT="ntx-wit-${folder}-${VERSION}"

  rm -rf "$TMP_WIT_ROOT/$ARCHIVE_ROOT"
  mkdir -p "$TMP_WIT_ROOT/$ARCHIVE_ROOT/wit"
  # Since each tarball is already per-folder, avoid an extra '<folder>/' layer.
  cp -a "$src_dir/." "$TMP_WIT_ROOT/$ARCHIVE_ROOT/wit/"

  tar -czf "$DIST_DIR/ntx-wit-${folder}-${VERSION}.tar.gz" -C "$TMP_WIT_ROOT" "$ARCHIVE_ROOT"
done

rm -rf "$TMP_WIT_ROOT"

echo "==> Generating checksums"
(
  cd "$DIST_DIR"
  sha256sum * > SHA256SUMS.txt
)

echo "==> Done. Assets in $DIST_DIR"
ls -la "$DIST_DIR"
