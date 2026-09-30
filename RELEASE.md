# Release Process

semver、可复现产物、零手工步骤 —— 产物全部由 CI 构建, 人只做决定。

## 版本策略

- **0.x 阶段**: 补丁/行为变化按 `0.MINOR.PATCH` 走, 破坏性变更 bump MINOR
  (0.x 语义, 详见 semver spec §4; 触发词/字段契约视为公共 API)。
- **1.0 起**: 严格 semver —— 破坏性 → MAJOR, 新字段/新能力 → MINOR,
  修复 → PATCH。
- 公共 API = `ShellCommands.json` 字段契约、触发词、存根元数据、CLI
  (`stub` 子命令 / `--version` / `-V` / `--help`)与 JSON-RPC 响应形状。

## 版本固化

构建期就把以下三样东西写进每个二进制 —— `quiver --version` 在任何
环境(无 stdin、无 catalog、无网络)都能回放:

```
Quiver 0.4.0 5f6d2c9 (release 2026-10-01T12:34:56Z)
  │        │    │        │         │
  │        │    │        │         └─ UTC ISO-8601, 注入 QUIVER_BUILD_TIME
  │        │    │        └─ PROFILE 注入, "release" / "debug"
  │        │    └─ GITHUB_SHA, 注入 QUIVER_GIT_SHA, dev 走 git rev-parse HEAD
  │        └─ Cargo.toml `version`
  └─ NAME, 来自 src/identity.rs
```

分辨率(优先级降序):

1. **`QUIVER_GIT_SHA`** 环境变量 —— CI 在 `cargo build` 之前显式注入,
   让产物字节与 tag 完全对齐, 不依赖宿主机有 `.git/`。
2. **`git rev-parse HEAD`** —— 本地 `make build` / `make windows`:
   `build.rs` 在 `.git/HEAD` 存在时直接调 git, dev 工作流零配置。
3. **`"unknown"`** —— `cargo-chef` recipe cook / 无 git 的镜像回退;
   `--version` 此时打印 `Quiver 0.4.0 (release unknown)`, Build
   字段从 stub 中省略(Wox UI 看不到 "unknown")。

存根(`stub` 子命令)的 JSON 也带 `Build` 字段, Wox 解析时会忽略未知
字段, 所以旧 Wox 不会因新增字段而拒绝加载。

## 发布步骤

1. **确认 CI 绿**(`main` 上 fmt/lint/test/smoke/MSRV/cross 全过)。
2. **更新版本与变更记录**:
   - `Cargo.toml` 的 `version`;
   - `git log --oneline <last-tag>..` 整理进 release notes(用户可观察的
     行为差异, 不是 commit 罗列)。
3. **打 tag**: `git tag -s vX.Y.Z -m "quiver vX.Y.Z"`(签署 tag)并推送。
4. **CI 出产物 + 草稿 release**: tag 触发 `.github/workflows/release.yml`,
   产出并附带校验和, **创建为 draft release**(owner 在 UI 审完才
   publish):
   - `quiver-x86_64-pc-windows-gnu.exe` (Windows, GNU ABI; 走 mingw-w64)
   - `quiver-x86_64-unknown-linux-gnu` (Linux, x86_64)
   - `quiver-aarch64-apple-darwin` (macOS, arm64; M1/M2/M3/M4)
   - `sha256sums.txt`

   macOS runner 是 GitHub-hosted `macos-14`(原生 arm64, 不走 qemu/osxcross);
   不出 Intel 版, Rosetta 用户装 arm64 二进制即可。

   `release.yml` 的 "Release" step 用 `gh release create --draft --generate-notes`,
   而不是任何第三方 action。Draft releases 对未认证 API 不可见——这
   是 GitHub 的正常行为, 不是漏报; 仓库 owner 在 Releases UI 能看到。

5. **发布说明 + publish**: 在 GitHub Releases 页面打开这条 draft, 审计
   自动生成的 release notes + 4 个附件列表, 然后点 "Publish release"。
   `gh release edit vX.Y.Z --draft=false` 等价的 CLI 命令也可以。
6. **dotfiles 联动**(本工作站): Windows 侧 `sync-windows-host.sh` 会拉取
   新 release; POSIX 侧 `da` 走本地构建 —— 无需额外动作。

### Re-tag(workflow 改完之后重出 release)

如果 release.yml 改过、bump-all 之后再推同一个 tag:
`release.yml` 现在的 "Release" step 会先 `gh release delete <tag> --cleanup-tags --yes`,
再 `gh release create`。这是为了: retry 同一 tag 时, 把旧 draft 清掉,
而不是 `gh release create: already exists` 让 workflow 红一次。
代价: **重新 push tag 时, draft 会被替换一次**——审的时候只看到最新
一次产物的 draft, 不会看到历次 draft 的残骸。

## 回滚

产物是单二进制, 回滚 = 装上一版 release 文件; catalog 向后兼容(新增字段
带默认值, 老二进制忽略新字段、新二进制容忍老字段), 无需迁移。

## 供应链审计(`cargo deny` vs `--locked`)

依赖更新走 [Dependabot](.github/dependabot.yml) 周更: 拉新版本
触发 CI 重 build; 任意一次红都要评审。`Cargo.lock` 提交入仓,
`cargo build --locked` 在 CI 每一个 job 强制使用仓内的 lock 记录。

**没有** 接 [`cargo deny`](https://embarkstudios.github.io/cargo-deny/)。
六个直接依赖全是单主仓 + 主流活跃 crate(`serde` / `serde_json` /
`tracing` / `tracing-subscriber` / `fuzzy-matcher` / `shellexpand`),
间接依赖通过 `--locked` 与 Dependabot 周更 + MSRV 闸门兜底;
`cargo deny` 的 `advisories` / `bans` / `sources` / `licenses` 四类
闸门在本仓的边际收益低于额外维护 `deny.toml` + 多一次 CI job 的成本。
当依赖数量扩张、出现 `unsafe` 依赖或引入 AGPL 等限制性许可时再引入,
届时 deny.toml 是单一入口。

## 校验和

所有 release 附件都有 `sha256sums.txt`; 下载后:

```sh
sha256sum -c sha256sums.txt --ignore-missing
```

## 供应链证明(SLSA Build L1)

CI 在出 sha256 后、推 release 前做**三层绑定**——任何一层不匹配则
release job 直接 `exit 1`:

1. **SHA-256** 写在 `sha256sums.txt`(同 release 附件)。本地
   `sha256sum -c sha256sums.txt --ignore-missing` 即可独立校验。
2. **Signed tag** — `git tag -s vX.Y.Z`, 本地 `git verify-tag vX.Y.Z`
   校验 tag-publisher 身份。
3. **Baked identity** — `strings dist/quiver-* | grep -qF $GITHUB_SHA`
   校验 tag commit 已烤进每个 binary 的 `--version` 输出与 stub
   `Build` 字段。`strings`(1) 而非执行 —— 因为 `release` job 在
   ubuntu 上跑, 不能执行 macOS arm64 / Windows PE。

三层加起来满足 **SLSA Build L1**(provenance 强于 "由谁、什么 commit、
什么 builder")。

**L2** 在此之上要求签名 in-toto attestation 绑定到 workflow 的
OIDC identity。GitHub 一等公民 `actions/attest-build-provenance@v2`
本可提供, 但它在 personal repo 上用默认 `GITHUB_TOKEN` 会
`Resource not accessible by integration`(attestation 端点要求
显式 PAT 或 OIDC token)。给一个 PAT 是一笔独立的安全评审,
**目前不在本仓的复杂度预算内**——升级路径是 `release.yml` 的
一个 `uses: actions/attest-build-provenance@v2` step + 一份 PAT
`attestations:write` 权限, 是个明确的"何时做, 谁来做"留白项。
