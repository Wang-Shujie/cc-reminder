# 架构优化提案(v2-issues「项目架构整体优化」,供裁决)

日期:2026-08-27 · 状态:提案,未实施 · 原则:演进不重写;每条独立可裁、可单独成轮。

## 1. 前端数据请求层(优先级最高,收益面最大)

**现状**:每个页面手写 加载/错误/刷新 三态(OverviewPage/HistoryPage/AgentsPage/ChannelsPage/ProjectsPage/SettingsPage 各一套 useEffect + state,约 6 处重复形态);core:// 事件已驱动各自刷新,但无统一缓存。

**提案**:一个 `useCoreQuery(key, fetcher)`(~60 行,零依赖):内置 loading/error/data 三态 + 订阅给定 core:// 事件自动重取 + 卸载防漏写。逐页替换(每页一提交,测试不变断言即绿)。**不引库**(TanStack Query 等在此规模是杀鸡用牛刀)。

**裁点**:是否接受"零依赖自研 60 行"路线。

## 2. 组件目录重组

**现状**:按 v1 任务命名的目录(hooks/ channels/ agents/ projects/ …)与文件名 HookRulesPage 等历史语义混杂;"hooks" 目录在 React 语境下误导。

**提案**:扁平化 `src/pages/{workbench,rules,projects,integrations,settings,channels,agents,overview,history,onboarding}/` + `src/shell/` + `src/lib/`。纯移动 + import 路径更新,零逻辑变更,一次提交(脚本化 sed + tsc 兜底)。

**裁点**:时机——建议与 1 同轮(动文件少时改逻辑更省)或延后到下次大动。

## 3. Rust 侧

**现状**:commands/rules.rs ~750 行承载读写+装饰+测试;CoreState 11 个平铺字段。

**提案**(三条独立):
- commands/rules.rs 拆 views(装饰)/io(读写)/tests 三文件,mod 收口;
- CoreState 分组为 `storage: StorageHandles`、`runtime: RuntimeHandles`(cancel/worker/autostart/events/resources)两结构,构造点仅 lib.rs 与测试;
- docs/architecture.md:一页分层图(WebView ↔ commands ↔ repositories/worker ↔ FS/OS 安全存储),标注依赖方向与禁止逆向(如 repositories 不得 import commands)。

**裁点**:CoreState 分组会触碰所有 `state.x` 引用(机械但面广),可只做前两条。

## 4. HookRuleDrawer 重组(930 行)

**现状**:单文件六段配置(启用/目标/过滤/隐私/投递/静默)+ 预览 + 测试,全部一个组件。

**提案**:按段拆六个子表单组件 + 一个 drawer 壳(状态提升到壳),每段独立测试;行为与视觉零变化(截图基线不变即验收)。

**裁点**:独立成轮(纯重构,无功能收益,仅可维护性)。

## 建议实施顺序

1(请求层)→ 4(Drawer)→ 2(目录)→ 3(Rust 拆分+文档)。1+4 是日常改动最常触碰的痛点;2、3 是结构性清理,可推迟。
