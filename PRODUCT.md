# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(Tauri 2 桌面壳承载的 web 渲染界面;设计语言按 web 走,不做逐 OS 自适应。)

## Users

- 主要受众:**公开分发**的使用 Claude Code / Codex 的开发者(GitHub Releases 分发);陌生人需要第一眼信任并快速上手。
- 开发者本人是日常重度用户与首个验收人;配置/诊断等高级场景面向熟练用户。

## Product Purpose

CC Reminder 通过 Claude Code 与 Codex 的生命周期 Hook 捕获事件,在本地完成规则匹配、隐私过滤与排队,把通知发送到钉钉/企业微信群机器人。成功 = 用户不看终端也能在群里第一时间知道:任务完成、等待确认、需要授权、投递失败。

## Positioning

- **单向出站通知**:审批、停止等操作留在 Agent 原有流程,不反向控制。
- **本地处理**:事件在本地脱敏,凭据只进 OS 安全存储,无遥测、无云端依赖。
- 邻近产品(群机器人网关、CI 通知器)不可truthfully复制:与 Agent Hook 生命周期的深度集成 + 本地隐私边界。

## Operating Context

- 使用场景:开发者桌面,与 Claude Code/Codex 终端并行;通知终点是钉钉/企微群。
- 语言:zh-CN 权威,en 精确镜像(类型强制)。
- 桌面最小窗口 960×640;深/浅/system 三主题。
- 首次使用有五步引导(检测 → 装 Hook → 渠道 → 默认规则 → 测试发送)。

## Capabilities and Constraints

- 信息架构(v2 已定):4 目的地——工作台(状态概览/通知记录)、通知规则(规则表/项目管理)、集成(来源/去向)、设置;页内 TabBar 子导航。
- 必须保留的功能、术语与流程:健康问题→修复页跳转、失败任务下钻、渠道凭据不回显、诊断导出、更新流程。
- a11y 硬门禁:e2e axe serious/critical=0、全键盘可达、`.cc-focusable` 焦点可见、截图基线随视觉变更同步更新。
- 原生能力:文件夹选择、自启动、更新器、安全存储——不模拟这些控件。
- **2026-08-26 用户裁决:界面全面重设计(非精修)**,旧界面只作证据与反面参考;信息架构与功能不动。

## Brand Commitments

- 名称:CC Reminder。
- **视觉基调:简约/商务**(用户 2026-08-26 steer 并经 safer 档确认);2026-08-26 选定视觉世界「导视标识系统」(wayfinding signage)。
- 无标志/插画资产。

## Evidence on Hand

- README、docs/operations.md(运维手册)、docs/superpowers/ 设计与计划文档。
- e2e 截图基线(tests/e2e/app.spec.ts-snapshots/)是真实渲染的界面证据。
- 无用户证言、下载数据、案例——未来工作不得虚构。

## Product Principles

1. **状态先于装饰**:打开即回答"现在正常吗";任何视觉决策不得拖慢扫读。
2. **本地与隐私是信任基石**:界面表述与流程要显式强化"数据不出本机"的信任感。
3. **配置一次,长期旁观**:工具的常态是背景运行;界面为偶发检查与故障修复而设计,不为沉浸浏览。
4. **键盘与读屏是一等公民**:a11y 门禁不因视觉野心妥协。
5. **双语同一品质**:zh 与 en 不是主次关系,布局须同时容纳两种文字长度。

## Accessibility & Inclusion

延续既有硬门禁:axe serious/critical=0、键盘全可达、焦点可见、200% 缩放不丢内容;深浅两主题对比度均需达标。
