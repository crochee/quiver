# Security Policy

## 支持版本

| 版本 | 状态 |
| :--- | :--- |
| `main`(最新 tag) | 支持 |
| 更早版本 | 不支持 —— 升级到最新 release |

## 报告漏洞

**请不要用公开 issue / PR / 讨论区报告安全问题。**

请通过 GitHub 的
[私有漏洞报告](https://github.com/crochee/quiver/security/advisories/new)
提交; 若不可用, 联系仓库维护者(crochee)。请在报告里包含:

- 受影响版本/commit;
- 复现步骤或 PoC;
- 你评估的影响面。

**承诺**: 72 小时内确认收到; 修复与披露节奏与报告者协商(通常 90 天内)。
修复发布后会在 release notes 里致谢(除非要求匿名)。

## 范围与威胁模型

Quiver 是**本机、单用户**的 Wox 脚本插件, 明确在范围内/外的:

**范围内**
- `ShellCommands.json` 解析路径的内存安全(serde_json 承担解析);
- 恶意构造的 stdin JSON-RPC 请求导致的 panic / 无界资源消耗;
- capture 预览路径的 Markdown/转义处理。

**范围外(设计使然, 文档已声明)**
- catalog 是**用户本人**的可信文件 —— `{query}`/`$@`/`$N` 是纯文本替换,
  与上游 Wox Custom Commands 同一注入面; 把不可信输入喂进 alias 是使用
  错误, 不是漏洞;
- Wox 宿主自身的安全问题 —— 报给 Wox 上游。
