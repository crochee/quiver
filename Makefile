# Makefile — Quiver, a native Wox script plugin.
#
# The plugin itself is pure Rust; nothing here runs from chezmoi. This file is
# only the **manual build entry point** the docs and the sync hook refer to.
# 维护流程(CNCF 式: fmt/lint/test/smoke/MSRV/cross 全在 CI)见 CONTRIBUTING.md;
# catalog 契约的权威文档在 ~/.dotfiles/docs/wox/README.md §4.3.
#
#   make            WSL: 交叉编译 Windows PE（docker 多阶段, 见下）
#                   其他宿主: 本机 cargo build
#   make windows    显式走 docker 多阶段交叉编译（buildx --output 导出）
#   make test-windows  Windows 目标测试 PE（容器编 → WSL interop 跑, 仅 WSL）
#   make linux-test    容器内跑 Linux 单元测试（无需宿主 Rust）
#   make build      本机 cargo build --release（永远本机目标, 不交叉）
#   make test       单元测试（本机目标）
#   make smoke      离线冒烟（examples/quiver-smoke.sh；不需要 Wox）
#   make lint       rustfmt --check + clippy -D warnings
#   make fmt        应用 rustfmt
#   make clean      删除 target/ 与 dist/
#   make help       列出以上目标
#
# 交叉编译**不需要宿主装任何东西**: Dockerfile 自带 Rust 工具链与 mingw-w64
# 链接器, 多阶段 + cargo-chef + BuildKit cache mount 让依赖层跨构建复用
# (模式同 ~/workspace/cim/Dockerfile.server). 宿主只需要 docker **buildx**
# (Docker 23+ 自带). buildx 时代没有 bind-mount, Git-Bash/MSYS 的路径改写
# 也不再是问题 —— 但 windows 系目标仍建议在 WSL/Linux 跑, interop 执行
# 测试 PE 只有 WSL 有.

SHELL := /bin/sh
CARGO ?= cargo
DOCKER ?= docker
WINDOWS_IMAGE ?= quiver-builder
DIST_DIR ?= dist
DIST_TEST_DIR ?= dist-test

# --- target selection ------------------------------------------------------
# WSL counts as a Windows host: the Wox that receives the hotkey is Wox.exe, so
# the artifact has to be a Windows PE. Native Linux/macOS and Git-Bash/MSYS/
# Cygwin need no --target (their host *is* the target).
UNAME_S := $(shell uname -s 2>/dev/null)
UNAME_R := $(shell uname -r 2>/dev/null | tr 'A-Z' 'a-z')

ifeq ($(TARGET),)
  ifneq (,$(findstring microsoft,$(UNAME_R)))
    TARGET := x86_64-pc-windows-gnu
  endif
endif

HOST_IS_WINDOWS := $(if $(filter MINGW% MSYS% CYGWIN%,$(UNAME_S)),1,)
IS_WINDOWS := $(if $(findstring windows,$(TARGET)),1,$(HOST_IS_WINDOWS))

# On WSL the binary Wox loads is a Windows PE, so the default goal is the
# container cross build. Everywhere else the host builds for itself.
ifeq ($(TARGET),x86_64-pc-windows-gnu)
  ifeq ($(HOST_IS_WINDOWS),)
    .DEFAULT_GOAL := windows
  else
    .DEFAULT_GOAL := build
  endif
else
  .DEFAULT_GOAL := build
endif

.PHONY: build test test-windows linux-test smoke lint fmt clean help \
        windows windows-image

# --- cross compile (docker buildx, 多阶段) ---------------------------------
# 只重建构建镜像定义本身(通常不需要: buildx 按层缓存自动判断).
windows-image:
	$(DOCKER) buildx build --target artifact -t $(WINDOWS_IMAGE) .

# 多阶段构建 + 本地导出: dist/quiver.exe. 无 bind-mount, 无 root-owned
# target/, 依赖层与 registry 缓存由 BuildKit 持有.
windows:
	@mkdir -p $(DIST_DIR)
	$(DOCKER) buildx build \
	  --target artifact \
	  --output type=local,dest=$(DIST_DIR) \
	  -t $(WINDOWS_IMAGE) .
	@printf 'artifact: %s/quiver.exe\n' '$(DIST_DIR)'

# Cross-target tests: the PE test binary is *built* in the container (no host
# linker) and *executed* on the host through WSL interop, so the cfg(windows)
# assertions really run. WSL only: a native Linux box has no interop.
test-windows:
	@mkdir -p $(DIST_TEST_DIR)
	$(DOCKER) buildx build \
	  --target test-artifact \
	  --output type=local,dest=$(DIST_TEST_DIR) \
	  -t $(WINDOWS_IMAGE)-test .
	./$(DIST_TEST_DIR)/quiver-test.exe

# Linux-target unit tests inside the container (for hosts without Rust).
linux-test:
	$(DOCKER) buildx build --target linux-test .

# --- native build ----------------------------------------------------------
# Always host-native: a cross target needs a linker the host usually lacks
# (WSL has no x86_64-w64-mingw32-gcc), and `make windows` owns cross builds.
build:
	@printf 'target:   %s\n' '$(if $(TARGET),$(TARGET),<host default>)'
	$(CARGO) build --release

test:
	$(CARGO) test --release

# Offline smoke: drives the JSON-RPC surface against examples/ShellCommands.json
# without Wox. Useful after editing src/ — proves the wiring (binary, catalog
# loader, stub renderer, fuzzy ranking, capture gate, action path) still holds.
# Requires python3 only for the harness's JSON field extractor.
#
# Always builds the **host-native** binary, even on WSL where the default goal
# is the Windows PE — the harness spawns the binary as a Linux ELF; the cross
# PE would need interop and cannot read the POSIX WOX_DIRECTORY_USER_DATA path.
HOST_ARTIFACT := target/release/quiver$(if $(HOST_IS_WINDOWS),.exe,)
smoke:
	@printf 'building host-native %s for smoke (default goal on WSL is the cross PE)\n' '$(HOST_ARTIFACT)'
	$(CARGO) build --release
	WOX_QUIVER_EXE='$(CURDIR)/$(HOST_ARTIFACT)' '$(CURDIR)/examples/quiver-smoke.sh'

lint:
	$(CARGO) fmt --check
	$(CARGO) clippy --release --all-targets -- -D warnings

fmt:
	$(CARGO) fmt

clean:
	$(CARGO) clean
	rm -rf $(DIST_DIR) $(DIST_TEST_DIR)
	@printf 'removed target/ and dist*/\n'

help:
	@printf '%s\n' \
	  'make                      WSL: docker 多阶段交叉编译 Windows PE / 其他宿主: 本机 cargo build' \
	  'make windows              docker buildx 多阶段交叉编译 → dist/quiver.exe（无需宿主 rust/mingw）' \
	  'make test-windows         Windows 测试 PE（容器编 + interop 跑, 仅 WSL）' \
	  'make linux-test           容器内跑 Linux 单元测试（无需宿主 Rust）' \
	  'make windows-image        只重建 docker 构建镜像定义' \
	  'make build                本机 cargo build --release（永远本机目标）' \
	  'make test                 单元测试（本机目标）' \
	  'make smoke                离线冒烟（examples/quiver-smoke.sh，无需 Wox）' \
	  'make lint                 rustfmt --check + clippy -D warnings' \
	  'make fmt                  应用 rustfmt' \
	  'make clean                删除 target/ 与 dist*/' \
	  'make help                 列出以上目标'
	@printf 'detected: uname=%s kernel=%s -> default=%s\n' \
	  '$(UNAME_S)' '$(UNAME_R)' '$(if $(findstring microsoft,$(UNAME_R)),windows,build)'
