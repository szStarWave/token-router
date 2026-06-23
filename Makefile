# Flowy Router — common dev & ops targets
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

.PHONY: help build release release-dylib release-arm release-arm64 \
        test check clean install \
        start stop restart status \
        env setup stats stats-global stats-zh stats-global-zh

UNAME_S := $(shell uname -s 2>/dev/null || echo Windows)
ifeq ($(UNAME_S),Windows)
  DYLIB := $(TARGET)/release/token_router.dll
else ifeq ($(UNAME_S),Darwin)
  DYLIB := $(TARGET)/release/libtoken_router.dylib
else
  DYLIB := $(TARGET)/release/libtoken_router.so
endif

help:
	@echo "  build            Build debug CLI + library"
	@echo "  release          Build release CLI (native)"
	@echo "  release-dylib    Build release dynamic library for Electron"
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
	@echo "Options:"
	@echo "  CONFIG=path      Pass --config to token-router (e.g. CONFIG=example/config.toml)"

build:
	$(CARGO) build

release:
	$(CARGO) build --release

release-dylib: release
	@test -f "$(DYLIB)" || (echo "expected dylib at $(DYLIB)" && exit 1)
	@echo "Built $(DYLIB)"
	@echo "C header: ffi/token_router.h"
	@echo "Electron example: example/electron/"

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