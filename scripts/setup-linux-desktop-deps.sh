#!/usr/bin/env bash
# Install native dependencies for building/running the Tauri desktop app on Linux.
set -euo pipefail

if ! command -v apt-get >/dev/null 2>&1; then
  echo "This script currently supports Debian/Ubuntu (apt). Install manually:" >&2
  echo "  webkit2gtk-4.1, gtk-3, appindicator, librsvg2, build-essential" >&2
  exit 1
fi

sudo apt-get update
sudo apt-get install -y \
  build-essential \
  curl \
  wget \
  file \
  libssl-dev \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  flatpak \
  flatpak-builder

if ! flatpak remote-list --user 2>/dev/null | awk '{print $1}' | grep -qx 'flathub' \
  && ! flatpak remote-list 2>/dev/null | awk '{print $1}' | grep -qx 'flathub'; then
  echo "Adding Flathub remote (user scope)..."
  flatpak remote-add --if-not-exists --user flathub \
    https://dl.flathub.org/repo/flathub.flatpakrepo
fi

echo
echo "Optional: install the Flatpak runtime used by packaging/flatpak manifests:"
echo "  flatpak install -y --user flathub org.gnome.Platform//46 org.gnome.Sdk//46"
