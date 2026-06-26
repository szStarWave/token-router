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

# Override: make start CONFIG=example/config.toml
CONFIG      ?=
CONFIG_FLAG := $(if $(CONFIG),--config $(CONFIG),)

# Release binary if built; otherwise `cargo run --`.
ROUTER      = $(if $(wildcard $(RELEASE_BIN)),$(RELEASE_BIN),$(CARGO) run --)

UNAME_S := $(shell uname -s 2>/dev/null || echo Windows)
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
        ui-build tauri-dev tauri-build

help:
	@echo "  build            Build debug CLI + library"
	@echo "  release          Build release CLI (native)"
	@echo "  release-dylib    Build release dynamic library for Electron"
	@echo "  package-electron-win  Build + pack Windows x64 Electron bundle (DLL, header, docs, examples)"
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
	@echo "  ui-build         Build desktop web UI (Vite → desktop/dist)"
	@echo "  tauri-dev        Run Tauri desktop app (dev)"
	@echo "  tauri-build      Build Tauri desktop app (release)"
	@echo ""
	@echo "Options:"
	@echo "  CONFIG=path      Pass --config to token-router (e.g. CONFIG=example/config.toml)"

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
	@echo "Electron example: example/electron/"
	@echo "Windows Electron bundle: make package-electron-win"

# Windows x64 Electron integration: DLL + C header + docs + npm/electron-builder snippets + smoke test.
package-electron-win: release-dylib
ifeq ($(UNAME_S),Windows)
	@if not exist "packaging\electron-win32\README.md" (echo ERROR: missing packaging/electron-win32 templates && exit /b 1)
	@if exist "$(ELECTRON_WIN_PKG_DIR)" rmdir /S /Q "$(ELECTRON_WIN_PKG_DIR)"
	@if exist "$(ELECTRON_WIN_ZIP_PATH)" del /F /Q "$(ELECTRON_WIN_ZIP_PATH)"
	@if not exist "$(TARGET)\dist" mkdir "$(TARGET)\dist"
	@if not exist "$(ELECTRON_WIN_RES)" mkdir "$(ELECTRON_WIN_RES)"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\ffi" mkdir "$(ELECTRON_WIN_PKG_DIR)\ffi"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\config" mkdir "$(ELECTRON_WIN_PKG_DIR)\config"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\docs" mkdir "$(ELECTRON_WIN_PKG_DIR)\docs"
	@if not exist "$(ELECTRON_WIN_PKG_DIR)\example\smoke" mkdir "$(ELECTRON_WIN_PKG_DIR)\example\smoke"
	@copy /Y "$(DYLIB)" "$(ELECTRON_WIN_RES)\$(ELECTRON_WIN_DLL)" >nul
	@copy /Y "ffi\token_router.h" "$(ELECTRON_WIN_PKG_DIR)\ffi\" >nul
	@copy /Y "example\config.toml" "$(ELECTRON_WIN_PKG_DIR)\config\config.toml" >nul
	@copy /Y "incept.md" "$(ELECTRON_WIN_PKG_DIR)\docs\incept.md" >nul
	@copy /Y "incept.html" "$(ELECTRON_WIN_PKG_DIR)\docs\incept.html" >nul
	@copy /Y "packaging\electron-win32\README.md" "$(ELECTRON_WIN_PKG_DIR)\README.md" >nul
	@copy /Y "packaging\electron-win32\electron-builder.example.yml" "$(ELECTRON_WIN_PKG_DIR)\" >nul
	@copy /Y "packaging\electron-win32\package.ffi-rs.json" "$(ELECTRON_WIN_PKG_DIR)\example\package.ffi-rs.json" >nul
	@copy /Y "packaging\electron-win32\smoke-main.mjs" "$(ELECTRON_WIN_PKG_DIR)\example\smoke\main.mjs" >nul
	@copy /Y "packaging\electron-win32\smoke-package.json" "$(ELECTRON_WIN_PKG_DIR)\example\smoke\package.json" >nul
	@powershell -NoProfile -Command "Set-Content -Path '$(ELECTRON_WIN_PKG_DIR)\VERSION' -Value '$(VERSION)' -NoNewline"
	@powershell -NoProfile -Command "Compress-Archive -Path '$(ELECTRON_WIN_PKG_DIR)' -DestinationPath '$(ELECTRON_WIN_ZIP_PATH)' -Force"
	@echo "Packaged $(ELECTRON_WIN_PKG_DIR)"
	@echo "Archive  $(ELECTRON_WIN_ZIP_PATH)"
else
	@rm -rf "$(ELECTRON_WIN_PKG)" "$(ELECTRON_WIN_ZIP)"
	@mkdir -p "$(TARGET)/dist"
	@mkdir -p "$(ELECTRON_WIN_RES)"
	@mkdir -p "$(ELECTRON_WIN_PKG)/ffi" "$(ELECTRON_WIN_PKG)/config" "$(ELECTRON_WIN_PKG)/docs"
	@mkdir -p "$(ELECTRON_WIN_PKG)/example/smoke"
	@cp "$(DYLIB)" "$(ELECTRON_WIN_RES)/$(ELECTRON_WIN_DLL)"
	@cp ffi/token_router.h "$(ELECTRON_WIN_PKG)/ffi/"
	@cp example/config.toml "$(ELECTRON_WIN_PKG)/config/config.toml"
	@cp incept.md "$(ELECTRON_WIN_PKG)/docs/incept.md"
	@cp incept.html "$(ELECTRON_WIN_PKG)/docs/incept.html"
	@cp packaging/electron-win32/README.md "$(ELECTRON_WIN_PKG)/README.md"
	@cp packaging/electron-win32/electron-builder.example.yml "$(ELECTRON_WIN_PKG)/"
	@cp packaging/electron-win32/package.ffi-rs.json "$(ELECTRON_WIN_PKG)/example/package.ffi-rs.json"
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
	$(ROUTER) $(CONFIG_FLAG) gateway start

stop:
	$(ROUTER) $(CONFIG_FLAG) gateway stop

restart:
	$(ROUTER) $(CONFIG_FLAG) gateway restart

status:
	$(ROUTER) $(CONFIG_FLAG) gateway status

env:
	$(ROUTER) $(CONFIG_FLAG) env

setup:
	$(ROUTER) $(CONFIG_FLAG) setup

stats:
	$(ROUTER) $(CONFIG_FLAG) stats

stats-global:
	$(ROUTER) $(CONFIG_FLAG) stats --global

stats-zh:
	$(ROUTER) $(CONFIG_FLAG) stats --lang zh

stats-global-zh:
	$(ROUTER) $(CONFIG_FLAG) stats --global --lang zh

ui-build:
	npm --prefix desktop run build

tauri-dev:
	npm --prefix desktop run tauri:dev

tauri-build:
	npm --prefix desktop run build
	npm --prefix desktop run icons:generate
	npm --prefix desktop run tauri:build