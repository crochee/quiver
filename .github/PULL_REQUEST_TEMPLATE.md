<!--
谢谢! 先看 CONTRIBUTING.md —— 开发循环与硬性约定都在那里。
-->

## Motivation

<!-- 解决什么问题; 链接相关 issue (#N)。 -->

## User-visible behavior change

<!-- 改动前 vs 改动后, 用户能观察到什么差异。无则写 "none"。 -->

## Testing

<!-- 新增/修改了哪些断言; 为什么它们能抓住回归。CI 必绿:
     fmt + clippy -D warnings + test + smoke + MSRV。 -->

## Checklist

- [ ] `make fmt lint test smoke` 本地全绿
- [ ] 行为改动带测试; 文档(README / docs/wox-install.md)已同步
- [ ] 提交带 DCO 签核(`git commit -s`)
