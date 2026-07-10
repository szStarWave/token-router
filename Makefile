# Token Router — common dev & ops targets
#
# Usage:
#   make              # show help
#   make release      # build optimized binary (native)
#   make release-arm  # build release for ARM64 Linux
#   make test         # run tests
#   make start        # start gateway daemon

CARGO       ?= cargo
CROSS       := cross
BIN         := token-router
TARGET      ?= target
RELEASE_BIN := $(TARGET)/release/$(BIN)
DEBUG_BIN   := $(TARGET)/debug/$(BIN)

# Cross-compilation targets
ARM64_TARGET        := aarch64-unknown-linux-gnu
ARM64_RELEASE       := $(TARGET)/$(ARM64_TARGET)/release/$(BIN)
ARM64_MUSL_TARGET   := aarch64-unknown-linux-musl
ARM64_MUSL_RELEASE  := $(TARGET)/$(ARM64_MUSL_TARGET)/release/$(BIN)
MACOS_APP           := desktop/src-tauri/target/release/bundle/macos/Token Router.app
MACOS_DMG           := desktop/src-tauri/target/release/bundle/dmg/Token Router_$(DESKTOP_VERSION)_aarch64.dmg

# Override: make start HOME=/tmp/token-router-dev PORT=11080
HOME        ?=
PORT        ?=
HOME_FLAG   := $(if $(HOME),--home $(HOME),)
PORT_FLAG   := $(if $(PORT),--port $(PORT),)
ROUTER_FLAGS := $(HOME_FLAG) $(PORT_FLAG)

# Release binary if built; otherwise `cargo run --`.
ROUTER      = $(if $(wildcard $(RELEASE_BIN)),$(RELEASE_BIN),$(CARGO) run --)

ifeq ($(OS),Windows_NT)
UNAME_S := Windows
else
UNAME_S := $(shell uname -s 2>/dev/null || echo Unknown)
endif
ifeq ($(UNAME_S),Windows)
  DYLIB := $(TARGET)\release\token_router.dll
else ifeq ($(UNAME_S),Darwin)
  DYLIB := $(TARGET)/release/libtoken_router.dylib
else
  DYLIB := $(TARGET)/release/libtoken_router.so
endif

# Electron Windows x64 integration bundle (under cargo target/)
ELECTRON_WIN_PKG     := $(TARGET)/dist/token-router-electron-win32-x64
ELECTRON_WIN_ZIP     := $(TARGET)/dist/token-router-electron-win32-x64.zip
ifeq ($(UNAME_S),Windows)
ELECTRON_WIN_PKG_DIR := $(TARGET)\dist\token-router-electron-win32-x64
ELECTRON_WIN_RES     := $(ELECTRON_WIN_PKG_DIR)\resources\win32\x64
ELECTRON_WIN_ZIP_PATH := $(TARGET)\dist\token-router-electron-win32-x64.zip
else
ELECTRON_WIN_PKG_DIR := $(ELECTRON_WIN_PKG)
ELECTRON_WIN_RES     := $(ELECTRON_WIN_PKG)/resources/win32/x64
ELECTRON_WIN_ZIP_PATH := $(ELECTRON_WIN_ZIP)
endif
ELECTRON_WIN_DLL     := token_router.dll

ifeq ($(UNAME_S),Windows)
VERSION              := $(shell for /f "tokens=2 delims=@" %%A in ('$(CARGO) pkgid 2^>nul') do @echo %%A)
else
VERSION              := $(shell $(CARGO) pkgid 2>/dev/null | sed 's/.*@//' | tr -d '\r')
endif

.PHONY: help build release release-dylib release-arm release-arm64 \
        test check clean install \
        start stop restart status \
        env setup stats stats-global stats-zh stats-global-zh \
        package-electron-win clean-package-electron-win \
        ui-build tauri-dev tauri-build tauri-build-linux tauri-build-macos \
        setup-linux-desktop-deps setup-tauri-nsis flatpak-build \
        ota-stage build-ota ota-publish push version

help:
	@echo "  build            Build debug CLI + library"
	@echo "  release          Build release CLI (native)"
	@echo "  release-dylib    Build release dynamic library for Electron"
	@echo "  package-electron-win  Build + pack Windows x64 Electron integration bundle"
	@echo "  release-arm      Alias for release-arm64-musl (most portable)"
	@echo "  release-arm64    Build release for ARM64 Linux (glibc)"
	@echo "  release-arm64-musl  Build release for ARM64 Linux (musl, fully static)"
	@echo "  test             Run tests"
	@echo ""
	@echo "  env              Print resolved paths & config"
	@echo "  setup            Interactive upstream setup wizard"
	@echo "  stats            Session stats (English)"
	@echo "  stats-global     Global stats from stats.json"
	@echo "  stats-zh         Session stats (Chinese)"
	@echo ""
	@echo "  ui-build         Build desktop web UI (Vite → desktop/frontend/dist)"
	@echo "  tauri-dev        Run Tauri desktop app (dev)"
	@echo "  tauri-build      Build Tauri desktop app (release, auto-detect OS)"
	@echo "  tauri-build-linux  Build Tauri .deb on Linux"
	@echo "  setup-linux-desktop-deps  Install Linux build deps (Tauri + Flatpak)"
	@echo "  setup-tauri-nsis Prefetch NSIS toolchain for Windows Tauri bundling"
	@echo "  tauri-build-macos  Build Tauri desktop app on macOS (.app + .dmg)"
	@echo "  flatpak-build    Build Flatpak bundle from .deb (Linux)"
	@echo "  build-ota        Build release + stage OTA artifact (Windows/macOS/Linux)"
	@echo "  push             Stage and publish OTA artifact (needs MODELSCOPE_TOKEN)"
	@echo "  version X.Y.Z    Bump app version across manifests (e.g. make version 0.4.0 or VER=0.4.0)"
	@echo ""
	@echo "Options:"
	@echo "  HOME=path        Pass --home to token-router (e.g. HOME=/tmp/token-router-dev)"
	@echo "  PORT=number      Pass --port to token-router (e.g. PORT=11080)"
	@echo "  BUILD=1          Run build-ota before push"
	@echo "  OTA_REGION=CN|INTL   Override OTA region (default from VITE_EDITION)"
	@echo "  OTA_CHANNEL=flowy    Override OTA channel"
	@echo "  OTA_ENABLE_ACCOUNT=true|false"

build:
	$(CARGO) build

release:
	$(CARGO) build --release

release-dylib: release
ifeq ($(UNAME_S),Windows)
	@if not exist "$(DYLIB)" (echo expected dylib at $(DYLIB) && exit /b 1)
else
	@test -f "$(DYLIB)" || (echo "expected dylib at $(DYLIB)" && exit 1)
endif
	@echo "Built $(DYLIB)"
	@echo "C header: ffi/token_router.h"
	@echo "Electron integration: make package-electron-win"

# Windows x64 Electron integration: DLL + C header + docs + npm/electron-builder snippets + examples.
package-electron-win: release-dylib
ifeq ($(UNAME_S),Windows)
	@if not exist "packaging\electron-win32\README.md" (echo ERROR: missing packaging/electron-win32 templates && exit /b 1)
	@if exist "$(ELECTRON_WIN_PKG_DIR)" rmdir /S /Q "$(ELECTRON_WIN_PKG_DIR)"
	@if exist "$(ELECTRON_WIN_ZIP_PATH)" del /F /Q "$(ELECTRON_WIN_ZIP_PATH)"
	@if not exist "$(TARGET)\dist" mkdir "$(TARGET)\dist"
	@if not exist "$(ELECTRON_WIN_RES)" mkdir "$(ELECTRON_WIN_RES)"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\ffi" mkdir "$(ELECTRON_WIN_PKG_DIR)\ffi"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\bin" mkdir "$(ELECTRON_WIN_PKG_DIR)\bin"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\config" mkdir "$(ELECTRON_WIN_PKG_DIR)\config"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\docs" mkdir "$(ELECTRON_WIN_PKG_DIR)\docs"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\example\electron" mkdir "$(ELECTRON_WIN_PKG_DIR)\example\electron"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\example\smoke" mkdir "$(ELECTRON_WIN_PKG_DIR)\example\smoke"
	@copy /Y "$(DYLIB)" "$(ELECTRON_WIN_RES)\$(ELECTRON_WIN_DLL)" >nul
	@if exist "$(RELEASE_BIN)" copy /Y "$(RELEASE_BIN)" "$(ELECTRON_WIN_PKG_DIR)\bin\$(BIN).exe" >nul
	@copy /Y "ffi\token_router.h" "$(ELECTRON_WIN_PKG_DIR)\ffi\" >nul
	@copy /Y "packaging\electron-win32\incept.md" "$(ELECTRON_WIN_PKG_DIR)\incept.md" >nul
	@copy /Y "packaging\electron-win32\incept.html" "$(ELECTRON_WIN_PKG_DIR)\incept.html" >nul
	@copy /Y "packaging\electron-win32\incept.md" "$(ELECTRON_WIN_PKG_DIR)\docs\incept.md" >nul
	@copy /Y "packaging\electron-win32\incept.html" "$(ELECTRON_WIN_PKG_DIR)\docs\incept.html" >nul
	@copy /Y "example\config.toml" "$(ELECTRON_WIN_PKG_DIR)\config\config.toml" >nul
	@copy /Y "example\config.edge-only.toml" "$(ELECTRON_WIN_PKG_DIR)\config\config.edge-only.toml" >nul
	@copy /Y "example\config.minimal.toml" "$(ELECTRON_WIN_PKG_DIR)\config\config.minimal.toml" >nul
	@copy /Y "example\README.md" "$(ELECTRON_WIN_PKG_DIR)\example\README.md" >nul
	@copy /Y "packaging\electron-win32\README.md" "$(ELECTRON_WIN_PKG_DIR)\README.md" >nul
	@copy /Y "packaging\electron-win32\electron-builder.example.yml" "$(ELECTRON_WIN_PKG_DIR)\" >nul
	@copy /Y "packaging\electron-win32\package.ffi-rs.json" "$(ELECTRON_WIN_PKG_DIR)\example\package.ffi-rs.json" >nul
	@copy /Y "packaging\electron-win32\electron-main.mjs" "$(ELECTRON_WIN_PKG_DIR)\example\electron\main.mjs" >nul
	@copy /Y "example\electron\package.json" "$(ELECTRON_WIN_PKG_DIR)\example\electron\package.json" >nul
	@copy /Y "packaging\electron-win32\smoke-main.mjs" "$(ELECTRON_WIN_PKG_DIR)\example\smoke\main.mjs" >nul
	@copy /Y "packaging\electron-win32\smoke-package.json" "$(ELECTRON_WIN_PKG_DIR)\example\smoke\package.json" >nul
	@powershell -NoProfile -Command "Set-Content -Path '$(ELECTRON_WIN_PKG_DIR)\VERSION' -Value '$(VERSION)' -NoNewline"
	@powershell -NoProfile -Command "Compress-Archive -Path '$(ELECTRON_WIN_PKG_DIR)' -DestinationPath '$(ELECTRON_WIN_ZIP_PATH)' -Force"
	@echo "Packaged $(ELECTRON_WIN_PKG_DIR)"
	@echo "Archive  $(ELECTRON_WIN_ZIP_PATH)"
else
	@test -f "packaging/electron-win32/README.md" || (echo "ERROR: missing packaging/electron-win32 templates" && exit 1)
	@rm -rf "$(ELECTRON_WIN_PKG)" "$(ELECTRON_WIN_ZIP)"
	@mkdir -p "$(TARGET)/dist"
	@mkdir -p "$(ELECTRON_WIN_RES)"
	@mkdir -p "$(ELECTRON_WIN_PKG)/ffi" "$(ELECTRON_WIN_PKG)/bin" "$(ELECTRON_WIN_PKG)/config" "$(ELECTRON_WIN_PKG)/docs"
	@mkdir -p "$(ELECTRON_WIN_PKG)/example/electron" "$(ELECTRON_WIN_PKG)/example/smoke"
	@cp "$(DYLIB)" "$(ELECTRON_WIN_RES)/$(ELECTRON_WIN_DLL)"
	@test -f "$(RELEASE_BIN)" && cp "$(RELEASE_BIN)" "$(ELECTRON_WIN_PKG)/bin/token-router.exe" || true
	@cp ffi/token_router.h "$(ELECTRON_WIN_PKG)/ffi/"
	@cp packaging/electron-win32/incept.md packaging/electron-win32/incept.html "$(ELECTRON_WIN_PKG)/"
	@cp packaging/electron-win32/incept.md packaging/electron-win32/incept.html "$(ELECTRON_WIN_PKG)/docs/"
	@cp example/config.toml "$(ELECTRON_WIN_PKG)/config/config.toml"
	@cp example/config.edge-only.toml "$(ELECTRON_WIN_PKG)/config/config.edge-only.toml"
	@cp example/config.minimal.toml "$(ELECTRON_WIN_PKG)/config/config.minimal.toml"
	@cp example/README.md "$(ELECTRON_WIN_PKG)/example/README.md"
	@cp packaging/electron-win32/README.md "$(ELECTRON_WIN_PKG)/README.md"
	@cp packaging/electron-win32/electron-builder.example.yml "$(ELECTRON_WIN_PKG)/"
	@cp packaging/electron-win32/package.ffi-rs.json "$(ELECTRON_WIN_PKG)/example/package.ffi-rs.json"
	@cp packaging/electron-win32/electron-main.mjs "$(ELECTRON_WIN_PKG)/example/electron/main.mjs"
	@cp example/electron/package.json "$(ELECTRON_WIN_PKG)/example/electron/package.json"
	@cp packaging/electron-win32/smoke-main.mjs "$(ELECTRON_WIN_PKG)/example/smoke/main.mjs"
	@cp packaging/electron-win32/smoke-package.json "$(ELECTRON_WIN_PKG)/example/smoke/package.json"
	@echo "$(VERSION)" > "$(ELECTRON_WIN_PKG)/VERSION"
	@cd $(TARGET)/dist && rm -f token-router-electron-win32-x64.zip && zip -rq token-router-electron-win32-x64.zip token-router-electron-win32-x64
	@echo "Packaged $(ELECTRON_WIN_PKG)"
	@echo "Archive  $(ELECTRON_WIN_ZIP)"
endif

clean-package-electron-win:
ifeq ($(UNAME_S),Windows)
	@if exist "$(ELECTRON_WIN_PKG_DIR)" rmdir /S /Q "$(ELECTRON_WIN_PKG_DIR)"
	@if exist "$(ELECTRON_WIN_ZIP_PATH)" del /F /Q "$(ELECTRON_WIN_ZIP_PATH)"
else
	@rm -rf "$(ELECTRON_WIN_PKG)" "$(ELECTRON_WIN_ZIP)"
endif

release-arm: release-arm64

release-arm64:
	$(CROSS) build --release --target $(ARM64_TARGET)
	@echo "Built $(ARM64_RELEASE)"

start:
	$(ROUTER) $(ROUTER_FLAGS) gateway start

stop:
	$(ROUTER) $(ROUTER_FLAGS) gateway stop

restart:
	$(ROUTER) $(ROUTER_FLAGS) gateway restart

status:
	$(ROUTER) $(ROUTER_FLAGS) gateway status

env:
	$(ROUTER) $(ROUTER_FLAGS) env

setup:
	$(ROUTER) $(ROUTER_FLAGS) setup

stats:
	$(ROUTER) $(ROUTER_FLAGS) stats

stats-global:
	$(ROUTER) $(ROUTER_FLAGS) stats --global

stats-zh:
	$(ROUTER) $(ROUTER_FLAGS) stats --lang zh

stats-global-zh:
	$(ROUTER) $(ROUTER_FLAGS) stats --global --lang zh

# Desktop app (Tauri + frontend in desktop/frontend/)
DESKTOP     := desktop
PNPM        ?= pnpm

ui-build:
	$(PNPM) --dir $(DESKTOP)/frontend build

tauri-dev:
	$(PNPM) --dir $(DESKTOP) run tauri:dev

tauri-build:
	$(PNPM) --dir $(DESKTOP) run icons:generate
ifeq ($(UNAME_S),Windows)
	@powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-tauri-nsis.ps1
	$(PNPM) --dir $(DESKTOP) run tauri:build:win
else ifeq ($(UNAME_S),Darwin)
	$(PNPM) --dir $(DESKTOP)/frontend install
	$(PNPM) --dir $(DESKTOP) run tauri:build
else ifeq ($(UNAME_S),Linux)
	$(PNPM) --dir $(DESKTOP)/frontend install
	$(PNPM) --dir $(DESKTOP) run tauri:build:linux
else
	@echo "ERROR: tauri-build unsupported on $(UNAME_S)"
	@exit 1
endif

tauri-build-linux:
ifeq ($(UNAME_S),Linux)
	$(PNPM) --dir $(DESKTOP)/frontend install
	$(PNPM) --dir $(DESKTOP) run icons:generate
	$(PNPM) --dir $(DESKTOP) run tauri:build:linux
else
	@echo "ERROR: tauri-build-linux requires Linux"
	@exit 1
endif

setup-linux-desktop-deps:
	bash scripts/setup-linux-desktop-deps.sh

flatpak-build:
ifeq ($(UNAME_S),Linux)
	bash scripts/build-flatpak.sh
else
	@echo "ERROR: flatpak-build requires Linux"
	@exit 1
endif

setup-tauri-nsis:
ifeq ($(UNAME_S),Windows)
	@powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-tauri-nsis.ps1
else
	@echo "setup-tauri-nsis is Windows-only"
	@exit 1
endif

tauri-build-macos:
ifeq ($(UNAME_S),Darwin)
	$(PNPM) --dir $(DESKTOP)/frontend install
	$(PNPM) --dir $(DESKTOP) run icons:generate
	$(PNPM) --dir $(DESKTOP) run tauri:build
	@echo "Built $(MACOS_APP)"
	@test -f "$(MACOS_DMG)" && echo "DMG: $(MACOS_DMG)" || true
else
	@echo "ERROR: tauri-build-macos requires macOS (Darwin)"
	@exit 1
endif

# OTA publish (Windows release desktop → ModelScope)
OTA_CHANNEL          ?= flowy
OTA_ENABLE_ACCOUNT   ?= true
UV                   ?= uv
OTA_PUBLISH_SCRIPT   := scripts/publish_ota/publish.py
TAURI_PRODUCT_NAME   := Token Router
ifeq ($(UNAME_S),Windows)
DESKTOP_VERSION      := $(shell powershell -NoProfile -Command "$$l = (Select-String -Path 'desktop/src-tauri/Cargo.toml' -Pattern '^version = ' | Select-Object -First 1).Line; if ($$l) { $$l -replace 'version = \"','' -replace '\"','' } else { '0.0.0' }")
VITE_EDITION         := $(shell powershell -NoProfile -Command "$$v='domestic'; if (Test-Path 'desktop/frontend/.env') { Get-Content 'desktop/frontend/.env' | ForEach-Object { if ($$_ -match '^VITE_EDITION=(.+)$$') { $$v = $$matches[1].Trim('\"','''') } } }; $$v")
TAURI_NSIS_SETUP     := desktop\src-tauri\target\release\bundle\nsis\$(TAURI_PRODUCT_NAME)_$(DESKTOP_VERSION)_x64-setup.exe
OTA_STAGING_DIR      := $(TARGET)\ota
else
DESKTOP_VERSION      := $(shell grep -E '^version = ' desktop/src-tauri/Cargo.toml 2>/dev/null | head -1 | sed 's/version = "\(.*\)"/\1/' | tr -d '\r')
VITE_EDITION         := $(shell grep -E '^VITE_EDITION=' desktop/frontend/.env 2>/dev/null | head -1 | cut -d= -f2 | tr -d '\r"'"'"'' | tr '[:upper:]' '[:lower:]')
TAURI_NSIS_SETUP     := desktop/src-tauri/target/release/bundle/nsis/$(TAURI_PRODUCT_NAME)_$(DESKTOP_VERSION)_x64-setup.exe
OTA_STAGING_DIR      := $(TARGET)/ota
endif
ifndef OTA_REGION
ifeq ($(VITE_EDITION),international)
OTA_REGION           := INTL
else
OTA_REGION           := CN
endif
endif
export OTA_CHANNEL
export OTA_REGION
export OTA_ENABLE_ACCOUNT
export OTA_OS
ifeq ($(OTA_ENABLE_ACCOUNT),true)
OTA_ACCOUNT_DIR      := with_account
else
OTA_ACCOUNT_DIR      := without_account
endif
ifeq ($(UNAME_S),Windows)
OTA_OS               := windows
OTA_SETUP_SUFFIX     := -setup.exe
else ifeq ($(UNAME_S),Darwin)
OTA_OS               := macos
OTA_SETUP_SUFFIX     := .dmg
else ifeq ($(UNAME_S),Linux)
OTA_OS               := linux
OTA_SETUP_SUFFIX     := .flatpak
else
OTA_OS               := unknown
OTA_SETUP_SUFFIX     :=
endif
OTA_SETUP_NAME       := Token-Router-v$(DESKTOP_VERSION)-$(OTA_CHANNEL)-$(OTA_REGION)-$(OTA_ACCOUNT_DIR)$(OTA_SETUP_SUFFIX)
ifeq ($(UNAME_S),Windows)
OTA_STAGED_SETUP     := $(OTA_STAGING_DIR)\$(OTA_SETUP_NAME)
else
OTA_STAGED_SETUP     := $(OTA_STAGING_DIR)/$(OTA_SETUP_NAME)
endif
ifeq ($(UNAME_S),Windows)
OTA_PUBLISH_PLATFORM := windows
else ifeq ($(UNAME_S),Darwin)
OTA_PUBLISH_PLATFORM := macos
else ifeq ($(UNAME_S),Linux)
OTA_PUBLISH_PLATFORM := linux
else
OTA_PUBLISH_PLATFORM := unknown
endif
FLATPAK_ID           := com.tokenrouter.desktop
FLATPAK_BUNDLE       := $(TARGET)/flatpak/$(FLATPAK_ID)-$(DESKTOP_VERSION).flatpak

ifeq ($(UNAME_S),Windows)
ota-stage:
	@if not exist "$(TAURI_NSIS_SETUP)" (\
		echo ERROR: missing OTA artifact for windows & \
		echo Expected: $(TAURI_NSIS_SETUP) & \
		echo Run: make build-ota & \
		exit /b 1)
	@if not exist "$(OTA_STAGING_DIR)" mkdir "$(OTA_STAGING_DIR)"
	@copy /Y "$(TAURI_NSIS_SETUP)" "$(OTA_STAGED_SETUP)" >nul
	@echo $(OTA_STAGED_SETUP)> "$(OTA_STAGING_DIR)\.latest-artifact"
	@echo Staged $(OTA_STAGED_SETUP)
	@echo OTA_PLATFORM=windows
else
ota-stage:
	bash scripts/stage-ota.sh
endif

ifeq ($(UNAME_S),Windows)
build-ota: tauri-build ota-stage
else ifeq ($(UNAME_S),Darwin)
build-ota: tauri-build-macos ota-stage
else ifeq ($(UNAME_S),Linux)
build-ota: flatpak-build ota-stage
else
build-ota:
	@echo "ERROR: build-ota unsupported on $(UNAME_S)"
	@exit 1
endif

OTA_PUBLISH_CMD      = $(UV) run --with modelscope python $(OTA_PUBLISH_SCRIPT) \
		--channel $(OTA_CHANNEL) \
		--region-scope $(OTA_REGION) \
		--version v$(DESKTOP_VERSION) \
		--enable-account-system $(OTA_ENABLE_ACCOUNT) \
		--platform $(OTA_PUBLISH_PLATFORM) \
		--setup-path "$(OTA_STAGED_SETUP)"

ifeq ($(UNAME_S),Windows)
ota-publish: ota-stage
	@if "$(MODELSCOPE_TOKEN)"=="" (echo ERROR: set MODELSCOPE_TOKEN environment variable, e.g. $$env:MODELSCOPE_TOKEN = "your-token" && exit /b 1)
	@if "$(OTA_PUBLISH_PLATFORM)"=="" (echo ERROR: OTA publish unsupported on $(UNAME_S) && exit /b 1)
	@if "$(OTA_PUBLISH_PLATFORM)"=="unknown" (echo ERROR: OTA publish unsupported on $(UNAME_S) && exit /b 1)
	@if not exist "$(OTA_STAGED_SETUP)" (echo ERROR: missing $(OTA_STAGED_SETUP). Run: make ota-stage && exit /b 1)
	$(OTA_PUBLISH_CMD)
else
ota-publish: ota-stage
	@test -n "$(MODELSCOPE_TOKEN)" || (echo "ERROR: set MODELSCOPE_TOKEN environment variable" && exit 1)
	@test -n "$(OTA_PUBLISH_PLATFORM)" && test "$(OTA_PUBLISH_PLATFORM)" != "unknown" || (echo "ERROR: OTA publish unsupported on $(UNAME_S)" && exit 1)
	@test -f "$(OTA_STAGED_SETUP)" || (echo "ERROR: missing $(OTA_STAGED_SETUP). Run: make ota-stage" && exit 1)
	$(OTA_PUBLISH_CMD)
endif

ifeq ($(BUILD),1)
push: build-ota ota-publish
else
push: ota-publish
endif

# Bump version: make version 0.4.0  OR  make version VER=0.4.0
BUMP_VERSION := $(VER)
ifeq ($(BUMP_VERSION),)
BUMP_VERSION := $(firstword $(filter-out version,$(MAKECMDGOALS)))
endif
ifneq ($(filter version,$(MAKECMDGOALS)),)
$(eval $(filter-out version,$(MAKECMDGOALS)):;@:)
endif

version:
ifeq ($(BUMP_VERSION),)
	@echo ERROR: missing version. Usage: make version 0.4.0  OR  make version VER=0.4.0
	@exit 1
endif
	node scripts/bump_version.mjs $(BUMP_VERSION)