#!/usr/bin/env bash
# Resolve and copy the platform-specific OTA artifact into target/ota/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
STAGING_DIR="$TARGET/ota"

DESKTOP_VERSION="$(
  grep -E '^version = ' "$ROOT/desktop/src-tauri/Cargo.toml" 2>/dev/null \
    | head -1 \
    | sed 's/version = "\(.*\)"/\1/' \
    | tr -d '\r'
)"
OTA_CHANNEL="${OTA_CHANNEL:-flowy}"
OTA_REGION="${OTA_REGION:-CN}"
OTA_ENABLE_ACCOUNT="${OTA_ENABLE_ACCOUNT:-true}"
if [[ "$OTA_ENABLE_ACCOUNT" == "true" ]]; then
  OTA_ACCOUNT_DIR="with_account"
else
  OTA_ACCOUNT_DIR="without_account"
fi

if [[ -z "${VITE_EDITION:-}" && -f "$ROOT/desktop/frontend/.env" ]]; then
  VITE_EDITION="$(
    grep -E '^VITE_EDITION=' "$ROOT/desktop/frontend/.env" 2>/dev/null \
      | head -1 \
      | cut -d= -f2 \
      | tr -d '\r"'"'"'' \
      | tr '[:upper:]' '[:lower:]'
  )"
fi
if [[ "${VITE_EDITION:-}" == "international" ]]; then
  OTA_REGION="INTL"
fi

resolve_platform() {
  if [[ -n "${OTA_OS:-}" ]]; then
    echo "$OTA_OS"
    return
  fi
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*|Windows*) echo "windows" ;;
    Darwin) echo "macos" ;;
    Linux) echo "linux" ;;
    *) echo "unknown" ;;
  esac
}

PLATFORM="$(resolve_platform)"
case "$PLATFORM" in
  windows)
    SOURCE="$ROOT/desktop/src-tauri/target/release/bundle/nsis/Token Router_${DESKTOP_VERSION}_x64-setup.exe"
    DEST_NAME="Token-Router-v${DESKTOP_VERSION}-${OTA_CHANNEL}-${OTA_REGION}-${OTA_ACCOUNT_DIR}-setup.exe"
    ;;
  macos)
    SOURCE="$(ls -1 "$ROOT/desktop/src-tauri/target/release/bundle/dmg/"*.dmg 2>/dev/null | head -1 || true)"
    DEST_NAME="Token-Router-v${DESKTOP_VERSION}-${OTA_CHANNEL}-${OTA_REGION}-${OTA_ACCOUNT_DIR}.dmg"
    ;;
  linux)
    SOURCE="$TARGET/flatpak/com.tokenrouter.desktop-${DESKTOP_VERSION}.flatpak"
    DEST_NAME="Token-Router-v${DESKTOP_VERSION}-${OTA_CHANNEL}-${OTA_REGION}-${OTA_ACCOUNT_DIR}.flatpak"
    ;;
  *)
    echo "ERROR: unsupported platform for OTA staging: ${OTA_OS:-$(uname -s)}" >&2
    exit 1
    ;;
esac

if [[ -z "${SOURCE:-}" || ! -f "$SOURCE" ]]; then
  echo "ERROR: missing OTA artifact for $PLATFORM" >&2
  echo "Expected: ${SOURCE:-<unknown>}" >&2
  case "$PLATFORM" in
    windows) echo "Run: make build-ota" >&2 ;;
    macos) echo "Run: make build-ota  (or make tauri-build-macos)" >&2 ;;
    linux) echo "Run: make build-ota  (or make flatpak-build)" >&2 ;;
  esac
  exit 1
fi

mkdir -p "$STAGING_DIR"
DEST="$STAGING_DIR/$DEST_NAME"
cp -f "$SOURCE" "$DEST"
printf '%s\n' "$DEST" > "$STAGING_DIR/.latest-artifact"
echo "Staged $DEST"
echo "OTA_PLATFORM=$PLATFORM"
