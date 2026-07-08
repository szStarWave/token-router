#!/usr/bin/env bash
# Build a local Flatpak bundle from the Tauri .deb output.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FLATPAK_DIR="$ROOT/packaging/flatpak"
BUILD_DIR="$FLATPAK_DIR/.build"
REPO_DIR="$BUILD_DIR/repo"
STAGING_DIR="$BUILD_DIR/flatpak-builder"
DEB_STAGING="$BUILD_DIR/token-router-desktop.deb"
MANIFEST="$FLATPAK_DIR/com.tokenrouter.desktop.yml"
APP_ID="com.tokenrouter.desktop"

DESKTOP_VERSION="$(
  grep -E '^version = ' "$ROOT/desktop/src-tauri/Cargo.toml" 2>/dev/null \
    | head -1 \
    | sed 's/version = "\(.*\)"/\1/' \
    | tr -d '\r'
)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT_DIR="$TARGET/flatpak"
OUT_BUNDLE="$OUT_DIR/${APP_ID}-${DESKTOP_VERSION}.flatpak"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "ERROR: missing command: $1" >&2
    echo "Run: bash scripts/setup-linux-desktop-deps.sh" >&2
    exit 1
  fi
}

need flatpak
need flatpak-builder

ensure_flathub_remote() {
  if flatpak remote-list --user 2>/dev/null | awk '{print $1}' | grep -qx 'flathub'; then
    return 0
  fi
  if flatpak remote-list 2>/dev/null | awk '{print $1}' | grep -qx 'flathub'; then
    return 0
  fi
  echo "Adding Flathub remote (user scope)..."
  flatpak remote-add --if-not-exists --user flathub \
    https://dl.flathub.org/repo/flathub.flatpakrepo
}

ensure_gnome_runtime() {
  if flatpak info --user org.gnome.Platform//46 >/dev/null 2>&1; then
    return 0
  fi
  ensure_flathub_remote
  echo "Installing Flatpak runtime org.gnome.Platform//46 ..."
  flatpak install -y --user flathub org.gnome.Platform//46 org.gnome.Sdk//46
}

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "ERROR: flatpak-build requires Linux" >&2
  exit 1
fi

find_deb() {
  local pattern="$ROOT/desktop/src-tauri/target/release/bundle/deb/*.deb"
  local deb
  deb="$(ls -1 $pattern 2>/dev/null | head -1 || true)"
  if [[ -z "$deb" ]]; then
    echo "No .deb found at $pattern" >&2
    return 1
  fi
  printf '%s\n' "$deb"
}

if ! deb_path="$(find_deb)"; then
  echo "Building Tauri .deb first..."
  make -C "$ROOT" tauri-build-linux
  deb_path="$(find_deb)" || {
    echo "ERROR: Tauri build did not produce a .deb package" >&2
    exit 1
  }
fi

mkdir -p "$BUILD_DIR" "$OUT_DIR"
cp -f "$deb_path" "$DEB_STAGING"

ensure_gnome_runtime

rm -rf "$STAGING_DIR" "$REPO_DIR"
flatpak-builder \
  --force-clean \
  --user \
  --disable-cache \
  --repo="$REPO_DIR" \
  "$STAGING_DIR" \
  "$MANIFEST"

flatpak build-bundle "$REPO_DIR" "$OUT_BUNDLE" "$APP_ID"

echo "Built Flatpak bundle: $OUT_BUNDLE"
echo "Install locally: flatpak install --user --bundle $OUT_BUNDLE"
echo "Run: flatpak run $APP_ID"
