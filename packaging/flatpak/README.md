# Flatpak packaging (Flathub-ready)

App ID: `com.tokenrouter.desktop` (matches `desktop/src-tauri/tauri.conf.json`).

## Prerequisites

```bash
make setup-linux-desktop-deps
flatpak install -y flathub org.gnome.Platform//46 org.gnome.Sdk//46
```

## Local build

From the repo root:

```bash
make flatpak-build
# or full OTA staging pipeline:
make build-ota
```

Output:

| Artifact | Path |
|----------|------|
| Flatpak bundle | `target/flatpak/com.tokenrouter.desktop_<version>.flatpak` |
| OTA staging copy | `target/ota/Token-Router-v<version>-<channel>-<region>-<account>.flatpak` |

Install locally:

```bash
flatpak install --user --bundle target/flatpak/com.tokenrouter.desktop_*.flatpak
flatpak run com.tokenrouter.desktop
```

## OTA publish (ModelScope)

```bash
export MODELSCOPE_TOKEN=<token>
make build-ota
make push
# or build + publish in one step:
BUILD=1 make push
```

Manifest path on ModelScope: `{region}/{channel}/{with_account|without_account}/linux/latest.json`

## Submit to Flathub

1. Build release `.deb` on Linux: `make tauri-build-linux`
2. Upload the `.deb` to a GitHub Release (x86_64 and aarch64 if applicable)
3. Copy `packaging/flatpak/flathub/com.tokenrouter.desktop.yml` and `com.tokenrouter.desktop.metainfo.xml` into your Flathub PR branch
4. Replace `DEB_URL`, `DEB_SHA256`, `DEB_URL_AARCH64`, and `DEB_SHA256_AARCH64` in the manifest
5. Open a PR against [flathub/flathub](https://github.com/flathub/flathub) branch `new-pr`

See [Flathub submission docs](https://docs.flathub.org/docs/for-app-authors/submission/) and [Tauri Flatpak guide](https://v2.tauri.app/distribute/flatpak/).

## Sandbox permissions

The manifest grants network, home directory, tray icon, and single-instance DBus access required by the desktop app (embedded gateway, agent config writes, system tray).
