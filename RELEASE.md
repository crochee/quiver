# Release Process

semver、可复现产物、零手工步骤 —— 产物全部由 CI 构建, 人只做决定。

## 版本策略

- **0.x 阶段**: 补丁/行为变化按 `0.MINOR.PATCH` 走, 破坏性变更 bump MINOR
  (0.x 语义, 详见 semver spec §4; 触发词/字段契约视为公共 API)。
- **1.0 起**: 严格 semver —— 破坏性 → MAJOR, 新字段/新能力 → MINOR,
  修复 → PATCH。
- 公共 API = `ShellCommands.json` 字段契约、触发词、存根元数据、CLI
  (`--stub`)与 JSON-RPC 响应形状。

## 发布步骤

1. **确认 CI 绿**(`main` 上 fmt/lint/test/smoke/MSRV/cross 全过)。
2. **更新版本与变更记录**:
   - `Cargo.toml` 的 `version`;
   - `git log --oneline <last-tag>..` 整理进 release notes(用户可观察的
     行为差异, 不是 commit 罗列)。
3. **打 tag**: `git tag -s vX.Y.Z -m "quiver vX.Y.Z"`(签署 tag)并推送。
4. **CI 出产物**: tag 触发 `.github/workflows/release.yml`, 产出并附带
   校验和:
   - `quiver-x86_64-pc-windows-gnu.exe` (Windows)
   - `quiver-x86_64-unknown-linux-gnu` (Linux)
   - `sha256sums.txt`
5. **发布说明**: 把第 2 步的记录贴进 GitHub Release, 校验产物
   (`sha256sum -c`)后 publish。
6. **dotfiles 联动**(本工作站): Windows 侧 `sync-windows-host.sh` 会拉取
   新 release; POSIX 侧 `da` 走本地构建 —— 无需额外动作。

## 回滚

产物是单二进制, 回滚 = 装上一版 release 文件; catalog 向后兼容(新增字段
带默认值, 老二进制忽略新字段、新二进制容忍老字段), 无需迁移。

## 校验和

所有 release 附件都有 `sha256sums.txt`; 下载后:

```sh
sha256sum -c sha256sums.txt --ignore-missing
```
