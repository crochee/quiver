# examples/

离线样例 —— 不依赖 Wox, 用来:

- 看一份 catalog 长什么样 (`ShellCommands.json`, 7 条最小可演示 alias)
- 走一遍 JSON-RPC 协议, 不装启动器 (`quiver-smoke.sh`)

## 跑法

```sh
cd quiver
make build                                                # 出 target/release/quiver(.exe)
WOX_QUIVER_EXE=target/release/quiver ./examples/quiver-smoke.sh
# 或直接: make smoke (自动编 + 跑)
```

无 Wox, 无 Node, 无任何额外配置. 依赖只有 `python3` (脚本里 `json_get` 的
JSON 字段提取器) 和样例命令自身用到的 `ip`/`sed`/`touch` —— 脚本开头会
对缺失的依赖**响亮报错**, 而不是给出看不懂的 want/got 差异. 它把
`WOX_DIRECTORY_USER_DATA` 指向 `examples/`, 让插件读
`examples/ShellCommands.json` 而不是用户真实 catalog.

## 覆盖的契约

| 字段 / 行为 | 在样例里的演示 alias |
|---|---|
| `{query}` 占位符 | `echo` |
| `interpreter: bash` + `capture: true` (Linux) | `ip`, `now` |
| `interpreter: powershell` + `capture: true` (Windows-only) | `ip-win` |
| `silent: true` (回车后启动器立即隐藏; 执行**永远**是脱离式后台, `silent` 只管隐藏) | `wk` |
| `$@` (全部参数, 空格连接) | `k` |
| `$N` 与 `${N}` 内层 shell 转义 | `upper` |
| `workingDirectory` 字段 (本样本未演示, 见 `~/.dotfiles/docs/wox/README.md` §3) | — |

## 不要做的事

不要把 `examples/ShellCommands.json` 直接拷进 `~/.wox/wox-user/`. 这是冒烟 catalog, 名字
(`echo`/`upper`/`now`) 会和你真正的 alias 撞车; 真实 catalog 的权威版本由 chezmoi 管理,
见 `~/.dotfiles/home/dot_wox/wox-user/ShellCommands.json`.

## 扩展它

复制 `ShellCommands.json`, 在它的 `commands` 数组里加条目, 再跑
`quiver-smoke.sh` (插件**只读** `WOX_DIRECTORY_USER_DATA` 下的
`ShellCommands.json` 这一个文件名 —— 放进目录的其他 JSON 不会被读).
alias 大小写不敏感地重复时, `catalog::load()` 会**跳过后面的**并 warn 一行.

### 直接指定 catalog 文件(跳过目录拼接)

如果你的 catalog 不在 `WOX_DIRECTORY_USER_DATA` 目录下(共享 NFS 家目录、
per-project 配置仓、CI fixture 之类),可以绕过目录拼接,直接给完整路径:

```sh
QUIVER_PATH=/path/to/my.json examples/quiver-smoke.sh
```

这条路径优先级高于 `WOX_DIRECTORY_USER_DATA`,也高于平台默认目录。空字符串
等同于未设。

修改后跑 `cargo test --release` 验证 catalog loader 没回归 —— 排序与
默认键解析在 `src/catalog.rs::tests`; **capture 门**与 action 路径的断言
在 `src/protocol.rs::tests` (由 `capture_query_runs_synchronously_and_
binds_enter_to_clipboard` 起, 用真实子进程端到端验证); 装载校验(重复
alias/空 command/类型错字段/缺 commands 数组)在 `catalog::tests::loader`.

## CI / makefile 集成

脚本除 `python3` 外无依赖, `make smoke` 已挂好 (先编 host-native 二进制
再跑; 交叉编译出的 Windows PE 不会被自动选中 —— 经 WSL interop 它读不了
POSIX 路径).
