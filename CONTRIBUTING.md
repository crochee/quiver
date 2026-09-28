# Contributing to Quiver

感谢参与! 本仓按 CNCF 社区的通行做法维护 —— 门槛低、流程可预期、一切
以 CI 为准。先读 `README.md` 了解项目是什么, 再回来走流程。

## 开发环境

需要的只有: Rust(版本由 `rust-toolchain` 钉死, rustup 自动装)、docker
(仅交叉编译 Windows PE 时)、python3(冒烟脚本的 JSON 提取器)。

```sh
git clone <repo> && cd quiver
make fmt lint test smoke    # 改动后的全部护栏, 提交前必绿
```

## 开发循环

```sh
make build      # 本机 cargo build --release
make windows    # docker 多阶段交叉编译出 Windows PE (buildx)
make test       # 单元测试(含真实子进程的 capture/action 测试)
make smoke      # 离线端到端: stub/协议/capture/action/日志 37 项
make lint       # rustfmt --check + clippy -D warnings
```

改 `src/` 后: `make fmt lint test smoke` → 提交。改 `ShellCommands.json`
样例后: `make smoke` 即可(Wox fsnotify 热加载, 生产端同理)。

## 架构一览

| 模块 | 职责 | 改动前必读 |
|---|---|---|
| `main` | 入口: `--stub` 分支或 JSON-RPC serve | — |
| `identity` | 插件 id/触发词/图标/作者(单一来源) | — |
| `catalog` | `ShellCommands.json` 装载校验 + 排序 | 模块头 |
| `fuzzy` | `fuzzy-matcher::skim::SkimMatcherV2` 封装 (ASCII 兜底为子串匹配) | 模块头 |
| `protocol` | JSON-RPC wire types + query/action 处理 | 模块头与 Wox 源码引用 |
| `platform` | **全 crate 唯一**平台知识; 其余文件禁止非测试 `cfg` 平台门 | 模块头不变式 |
| `spawn` | 解释器分发 + 脱离式子进程 + capture 超时 | 模块头 |
| `stub` | Wox discovery-metadata 渲染 | 模块头 |
| `log` | EnvFilter + error floor | 模块头 |

硬性约定:

- `platform.rs` 之外**不得**出现生产代码平台 `cfg` / 平台专属标识符。
- 行为改动必须带测试; 测试改动必须能抓住真实的用户可见回归。
- 与 Wox 行为对齐的地方, 注释里都引了上游 Go 源码位置 —— 改前先对照。

## 提交规范

Conventional Commits(`feat:` / `fix:` / `docs:` / `refactor:` / `test:`
/ `chore:`), 一行说清"改了什么、为什么"。DCO 签核必备:

```sh
git commit -s    # 生成 Signed-off-by: 你 <邮箱>
```

提交即表示同意以 MIT 许可贡献, 且签署确认该贡献由你撰写/有权提交
(Developer Certificate of Origin, 与 CNCF 项目一致)。

## PR 检查单

PR 描述请覆盖:

1. **动机**: 解决什么问题(链接 issue)。
2. **行为差异**: 用户可观察的前后对比。
3. **测试**: 新增/修改了哪些断言, 为什么它们能抓住回归。

CI 必绿: fmt + clippy(-D warnings)+ 测试 + 冒烟 + MSRV(1.91)+ 交叉编译.
评审关注正确性、与 Wox 上游行为的一致性、测试是否测在点上。

## 发布

见 `RELEASE.md`(semver、tag、自动产物)。安全漏洞见 `SECURITY.md` ——
**不要**用公开 issue 报安全问题。

## 行为准则

Be excellent to each other: 对事不对人、保持专业、不接受骚扰性言论。
沿用 CNCF 社区准则的精神; 举报走仓库维护者私下渠道。
