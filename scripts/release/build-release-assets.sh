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
# Keep folder structure under `config/`.
(
  cd "$ROOT_DIR"
  zip -r "$DIST_DIR/ntx-config.zip" \
    config/app.yaml \
    config/config.yaml \
    config/resource \
    >/dev/null
)

echo "==> Packaging WIT bundles for wit-deps"

# Publish deps.toml as a standalone asset (reference for wit-deps-cli).
cp "$ROOT_DIR/plugins/wit/core/deps.toml" "$DIST_DIR/ntx-wit-deps.toml"

# Each WIT package folder becomes its own zip (e.g. ntx-wit-actions-executor.zip).
WIT_STAGING="$DIST_DIR/wit"
mkdir -p "$WIT_STAGING"
cp -R "$ROOT_DIR/component/wit"/* "$WIT_STAGING/"

(
  cd "$DIST_DIR"
  for dir in wit/*; do
    [[ -d "$dir" ]] || continue
    name="$(basename "$dir")"
    zip -r "$DIST_DIR/ntx-wit-${name}.zip" "$dir" >/dev/null
  done
)

rm -rf "$WIT_STAGING"

echo "==> Generating checksums"
(
  cd "$DIST_DIR"
  sha256sum * > SHA256SUMS.txt
)

echo "==> Done. Assets in $DIST_DIR"
ls -la "$DIST_DIR"
