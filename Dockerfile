# syntax=docker/dockerfile:1.7
#
# Quiver 的 Windows 交叉编译 —— 多阶段 + 缓存加速, 模式:
# cargo-chef 把第三方依赖固化成一独立层, 源码改动不会触发依赖重编;
# apt/registry 全部走 BuildKit cache mount (跨构建持久, 不进层).
# 裁剪成「产物导出」型: 本镜像不运行, 只出 quiver.exe.
#
#   make windows        → docker buildx build --target artifact --output=dist/
#   make test-windows   → buildx 出 Windows 测试 PE, 宿主 WSL interop 执行
#   make linux-test     → docker build --target linux-test (容器内跑单元测试)
#
# 跨平台宿主: builder 恒以 --platform=$BUILDPLATFORM 原生执行 (amd64/arm64
# 宿主都行, 不走 qemu), 交叉目标固定 x86_64-pc-windows-gnu —— 产物字节不随
# 宿主架构变. 升级工具链 = 改 RUST_IMAGE 默认值 + rust-toolchain 两处.
#
# 缓存: registry/git/apt 都是 BuildKit cache mount (跨构建持久, 不进层),
# 依赖层由 cargo-chef 的 recipe 只在 Cargo.toml/lock 变化时重编 —— 源码
# 改动不会重编第三方依赖.

ARG RUST_IMAGE=rust:1.98.1-slim-bookworm

# Release-time identity: `release.yml` passes the tag SHA (and a frozen
# build timestamp) so the Windows PE prints the exact commit it was cut
# from. Local `make windows` builds without these args fall back to
# `build.rs`'s `git rev-parse HEAD` path — which needs `.git/` to be in
# the build context (see `.dockerignore`).
ARG QUIVER_GIT_SHA=""
ARG QUIVER_BUILD_TIME=""

########## Stage 0: chef —— 工具链 + mingw + cargo-chef (可复用基底) ##########
FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS chef

# mingw-w64 提供 x86_64-w64-mingw32-gcc —— rustc 的 windows-gnu 目标调它链接.
# 该目标的 std 随 rustup 而非镜像, 所以显式 target add.
# apt cache mount 让 .deb 跨构建复用.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends mingw-w64; \
    rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu

# cargo-chef 一次性安装 (registry cache mount 复用下载).
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo install cargo-chef --locked
WORKDIR /work

########## Stage 1: planner —— 依赖清单 (recipe.json, ~KB 级) ##########
FROM chef AS planner
# .dockerignore 白名单保证构建上下文里只有这三个路径.
COPY Cargo.toml Cargo.lock ./
COPY src src
RUN cargo chef prepare --recipe-path recipe.json

########## Stage 2: cacher —— 依赖层 (Cargo.toml/lock 变化才重编) ##########
FROM chef AS cacher
COPY --from=planner /work/recipe.json .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo chef cook --release --target x86_64-pc-windows-gnu \
        --recipe-path recipe.json

########## Stage 3: builder —— 本仓库 crate (增量: 只编 src) ##########
FROM cacher AS builder
ARG QUIVER_GIT_SHA
ARG QUIVER_BUILD_TIME
ENV QUIVER_GIT_SHA=${QUIVER_GIT_SHA}
ENV QUIVER_BUILD_TIME=${QUIVER_BUILD_TIME}
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY build.rs ./
# `.git/HEAD` is the only git artefact build.rs needs to resolve the
# commit for local `make windows` runs (CI overrides via
# QUIVER_GIT_SHA and ignores this). `.dockerignore` whitelists it.
COPY .git/HEAD .git/HEAD
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release --locked --target x86_64-pc-windows-gnu \
    && cp target/x86_64-pc-windows-gnu/release/quiver.exe /quiver.exe

########## Stage 4: unittests —— Windows 测试 PE (WSL interop 执行) ##########
# --no-run 编出不执行; 测试二进制带 hash, 固定名拷出. 独立阶段: 它依赖
# builder 的依赖层但自己也要编 test 目标, 不污染 artifact 链.
FROM builder AS unittests
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY build.rs ./
COPY .git/HEAD .git/HEAD
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo test --release --locked --target x86_64-pc-windows-gnu --no-run \
    && t="$(ls -t target/x86_64-pc-windows-gnu/release/deps/quiver-*.exe | head -n1)" \
    && cp "$t" /quiver-test.exe

########## Stage 5: artifact —— scratch 导出层 (--output 用) ##########
FROM scratch AS artifact
COPY --from=builder /quiver.exe /quiver.exe

########## Stage 5b: test-artifact —— 测试 PE 导出层 ##########
FROM scratch AS test-artifact
COPY --from=unittests /quiver-test.exe /quiver-test.exe

########## Stage 6: linux-test —— 容器内跑 Linux 目标单元测试 ##########
# 供无宿主 Rust 的环境 (CI 可选); 失败即构建失败.
FROM chef AS linux-test
COPY Cargo.toml Cargo.lock ./
COPY src src
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo test --release --locked
