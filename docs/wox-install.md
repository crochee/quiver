# Quiver 安装与使用指南

> 30 秒版本: 装好 Wox → 把 `quiver` 二进制和存根放进 Wox 插件目录 → 在
> `~/.wox/wox-user/ShellCommands.json` 里写一条 alias → 按 `Alt+Space` 输入
> `qv` 回车。全文围绕这条链路展开, 排查不了再看后面几节。

## Quiver 是什么

一个 Wox 启动器的**原生脚本插件**: 把 `ShellCommands.json` 里的每条 alias
变成一支"箭" —— 关键字唤起、回车发射。Rust 单二进制, 无 Python / Node.js
运行时, Wox 每次按键直接 exec 它, stdin 收一条 JSON-RPC、stdout 回一条。

- 触发词: **`qv`** 或 **`quiver`**
- 仓库: 本仓 (`quiver/`), 上游为 dotfiles 子仓
- 完整 catalog 契约权威文档: `~/.dotfiles/docs/wox/README.md` §3-§4

## 前置要求

| 组件 | 要求 | 说明 |
| :--- | :--- | :--- |
| Wox | ≥ 2.0 | 脚本插件 runtime; 2.4.x 亦可 |
| 操作系统 | Windows / Linux / macOS | 二进制按平台编译; Windows 为主要目标 |
| `quiver` 二进制 | 见下"安装" | 单文件, 无依赖 |
| 存根文件 | 见下"安装" | Wox 靠它发现插件 |

## 安装

### 方式 A: dotfiles 自动安装(推荐, 本工作站)

上游 dotfiles 仓已内置两遍式流程(`da` = `chezmoi apply`):

1. `make`(WSL 下走 docker 交叉编译)产出 `target/x86_64-pc-windows-gnu/release/quiver.exe`;
2. `da` 把二进制经 `.chezmoiexternal.toml` 装到 `~/.local/bin/quiver`(POSIX)
   / GitHub release 拉取 `quiver.exe`(Windows 侧 `sync-windows-host.sh`),
   再用 `install-quiver-stub.sh` 渲染存根到 Wox 插件目录。

任何一环缺位都 warn-only 不 abort —— 缺哪补哪, 不会静默用旧二进制。
详见 `~/.dotfiles/docs/wox/README.md` §4.3。

### 方式 B: 手动安装(任何机器)

1. **取二进制**: 从 GitHub Release 下载对应平台的 `quiver`(Linux/macOS)
   或 `quiver.exe`(Windows), 或从源码构建:

   ```sh
   git clone <repo> && cd quiver
   make                # WSL: docker 交叉编译出 Windows PE
   # 或本机: make build
   ```

2. **放二进制**:

   | 平台 | 位置 | 说明 |
   | :--- | :--- | :--- |
   | Windows | `~\.wox\wox-user\plugins\scripts\` 同目录 | 与存根同名 `quiver.exe` |
   | Linux/macOS | `~/.local/bin/quiver`(须在 PATH 中) | 存根 shebang 按 PATH 找它 |

3. **渲染存根**(Wox 靠扫描插件目录的头部注释发现插件):

   ```sh
   quiver --stub windows > ~/.wox/wox-user/plugins/scripts/quiver      # Windows
   quiver --stub posix   > ~/.wox/wox-user/plugins/scripts/quiver.sh   # Linux/macOS
   ```

4. **重载**: Wox 用 fsnotify 监听插件目录, 放好文件即自动发现; 若没有,
   重启一次 Wox。

### 验证安装

```sh
echo '{"jsonrpc":"2.0","id":1,"method":"query","params":{"search":"now"}}' \
  | quiver          # 应输出一段 JSON, 含 "result":{"items":[...]}
```

按 `Alt+Space` 输入 `qv` —— 看到箭袋图标与示例条目即为成功。

## 第一个 alias(3 行上手)

编辑 `~/.wox/wox-user/ShellCommands.json`:

```json
{
  "commands": [
    { "alias": "yt", "command": "xdg-open \"https://youtube.com/results?search_query={query}\"" }
  ]
}
```

保存即生效(fsnotify)。`Alt+Space` → `yt 猫片` → 回车。就这么多。

## 字段速查表

| 字段 | 类型 | 默认 | 作用 |
| :--- | :--- | :--- | :--- |
| `alias` | string | 必填 | 触发名; 大小写不敏感; 重复时**第一条生效**并告警 |
| `command` | string | 必填 | 命令文本; 空缺/非字符串的条目被跳过并告警 |
| `interpreter` | string | 平台默认 | `bash`/`sh`/`zsh`(POSIX), `powershell`/`cmd`/`bash`/`python`/`node`(Windows); 其他值按可执行文件原样分发 |
| `workingDirectory` | string | 无(继承) | 子进程 cwd; 相对路径按 **home** 解析; 不存在则告警并继承 |
| `silent` | bool | `false` | `true` = 回车后启动器**立即隐藏**(命令照常后台跑); `false` = 保持打开 |
| `capture` | bool | `false` | `true` = 查询时同步执行, 输出进右侧预览, 回车=复制首行到剪贴板 |
| `enabled` | bool | `true` | `false` = 隐藏该条(不参与匹配) |
| `tags` | string[] | `[]` | 发现辅助; tag 命中永远排在 alias 命中之后 |
| `description` | string | 命令本身 | 结果行的副标题 |

顶层默认值: `defaultInterpreter(@windows/@darwin/@linux)`、
`defaultWorkingDirectory(同后缀)` —— 后缀键优先, 共享一份 catalog 的
多台机器各自取各自的。

## 占位符

| 写法 | 替换时机 | 展开为 |
| :--- | :--- | :--- |
| `{query}` | 查询时(alias 前缀命中) | alias 之后的**原始文本**(含空格, 不转义) |
| `$@` | 查询时(精确命中) | 全部参数, 空格连接 |
| `$1`…`$9` | 查询时(精确命中) | 第 N 个参数; 越界/缺省=空串 |
| `${1}`… | **不**替换 | 留给**内层 shell** 的位置参数(见下) |

**内层 shell 传参的惯用法**(外层替换会吞掉裸 `$N`, 所以加 braces 躲过,
再在结尾用裸 `$1` 把参数递进去):

```json
{ "alias": "upper",
  "command": "sh -c 'python3 -c \"import sys; print(sys.argv[1].upper())\" \"${1}\"' sh \"$1\"" }
```

`qv upper hello` → `HELLO`。

替换是**纯文本**的(与上游 Custom Commands 同一注入面): 参数里带引号/分号
会原样进入命令。需要安全引用时, 自己在 `command` 里加引号或用内层 sh。

## capture 与 silent 的语义

- **`capture: true`**: 在**查询时**(而非回车时)同步跑命令 —— 这是脚本插件
  能把输出给用户看的唯一窗口。右侧预览显示 stdout/stderr/退出码, 默认动作
  变为"复制 stdout 首行"。命令必须在秒级返回(插件本地 8 s 硬上限, 超时杀
  掉并在预览注明); 慢命令请改用 `silent: true`。
- **`silent: true`**: 只决定**回车后启动器藏不藏**。执行本身永远脱离式
  (detached)—— 命令总能跑完, 与启动器生命周期无关。

## 日志与排查

日志走 `tracing` → stderr, 被 Wox 收进它的日志文件。默认静默(仅 error):

```sh
QUIVER_LOG=info quiver       # info/warn/error
QUIVER_LOG=debug quiver      # 全量(含每条 query 与 catalog 路径)
QUIVER_LOG=quiver=debug,info # target 限定
RUST_LOG=...                 # QUIVER_LOG 未设时的兼容入口
```

拼错的指令**不会**静默 —— 一律回落到 error 底线(errors always loud)。

## Catalog 路径

按以下优先级定位 `ShellCommands.json`:

1. `QUIVER_PATH` —— **直接给文件路径**。优先级最高,常用于:
   共享 NFS 家目录(每台机器的家目录相同)、per-project 配置仓、CI fixture。
   ```sh
   QUIVER_PATH=/opt/shared/quiver/team.json quiver
   ```
2. `WOX_DIRECTORY_USER_DATA` —— Wox 标准的目录变量;Quiver 拼上
   `ShellCommands.json`。Wox 真实运行时就是这一档。
3. 平台默认目录 + `ShellCommands.json`(`$HOME/.wox/wox-user/ShellCommands.json`
   或 Windows 上的等价物)。

空字符串(`QUIVER_PATH=""`)等同于未设,落到下一档。

| 症状 | 先看 |
| :--- | :--- |
| `qv` 无任何条目 | catalog 文件是否存在/可解析(看 `QUIVER_LOG=debug` 里 `catalog_load path` 一行确认实际加载的路径); 缺 `commands` 数组会得到一条"catalog unavailable"错误行; 想换路径时设 `QUIVER_PATH=/path/to/file.json` |
| 条目没出现 | `enabled` 是否 `false`(字符串 `"false"` 无效, 须为 bool); alias 是否重复(第一条生效) |
| 回车没反应 | `command` 是否为空(空命令条目装载时已被跳过); `interpreter` 该平台是否存在 |
| 预览乱码 | Windows `cmd` 输出经 OEM 代码页回退解码; 若仍乱, 命令自身先 `chcp 65001` |
| 想确认到底执行了什么 | `QUIVER_LOG=debug` 看 `query received` / `spawn detached` |

离线自检(不需要 Wox): `make smoke` —— 37 项检查覆盖 stub/协议/capture/
action/日志全链路。

## 扩展

- **加命令**: 只改 `ShellCommands.json`(数据驱动, 不动二进制)。
- **换机器**: 同一份 catalog, 机器差异全部走 `@windows/@darwin/@linux`
  后缀键。
- **换/加解释器**: `src/platform.rs` 的 `INTERPRETERS` 表是唯一扩展点,
  单文件评审即可加平台/解释器 —— 见 `CONTRIBUTING.md`。

## 卸载

删掉插件目录里的存根(`quiver` / `quiver.sh`)与二进制, 再删
`~/.wox/wox-user/ShellCommands.json`(如不再用)。Wox 下次扫描自动 forget。
