# 主界面重组设计(7 页 → 4 页,v2 开场)

日期:2026-08-26
状态:已经用户确认(方案 B)
范围:纯前端 IA 重组 + 版本升级 2.0.0 + 遗留文档改造;不改任何后端/Rust 契约。

## 背景与目标

v1 主界面为 7 个平级导航(总览/Agent 集成/Hook 规则/通知渠道/项目/通知历史/设置),默认落在 Hook 规则页。用户反馈四个痛点:导航太散、功能归属不清且重叠、页内布局乱、默认页缺少工作台感。

用户对应用的真实心智只有三问:**现在正常吗(状态)→ 什么时候通知谁(规则)→ 接到哪、发到哪(集成)**,外加应用级设置。本设计据此重组。

## 1. 总体结构

| 新导航 | 默认 | 组成(全部复用现有组件) |
|---|---|---|
| 工作台 | ✅ 启动默认页 | Tab:状态概览(现 OverviewPage)+ 通知记录(现 HistoryPage) |
| 通知规则 | | Tab:规则表(现 HookRulesPage)+ 项目管理(现 ProjectsPage) |
| 集成 | | Tab:通知来源(现 AgentsPage)+ 通知去向(现 ChannelsPage) |
| 设置 | | 现状不变 |

统一模式:导航 = 4 个目的地,页内子导航 = 同一个 TabBar 组件(3 处复用)。不引入路由库,保持现有 state 导航。

## 2. 工作台页

- Tab:状态概览 | 通知记录。
- 状态概览面板 = 现 OverviewPage:队列指标条、健康问题列表(带修复跳转)、最近失败 5 条预览。
- 通知记录面板 = 现 HistoryPage:完整表格 + 筛选 + 详情抽屉 + 手动重试。
- "查看失败任务"从跨页跳转改为**页内切 Tab + 失败筛选**;`HistorySeed` 机制保留,从 AppShell 层下沉到工作台页内部。
- 健康问题修复跳转目标改映射:`agents → integrations`,`channels → integrations`,`hooks/projects → rules`,`queue/delivery/spool → 工作台通知记录 Tab`(issueCode 前缀判断逻辑同步调整)。
- 默认 Tab = 状态概览;Tab 选择不持久化,每次进页回到概览,保持"打开即见状态"。

## 3. 通知规则页

- Tab:规则表 | 项目管理。
- 规则表面板 = 现 HookRulesPage 原样:全局/项目 scope 单选(已有 `RulesScope` 与项目下拉,数据来自 `listProjects`)+ 表格 + 配置抽屉。
- 项目管理面板 = 现 ProjectsPage 原样:加根目录/别名/worktree 模式/解除关联。
- 不做交叉联动:不在规则工具栏里塞项目 CRUD,两个状态机保持隔离,diff 最小。

## 4. 集成页

- Tab:通知来源 | 通知去向。
- 来源面板 = 现 AgentsPage(Claude Code / Codex 检测、安装、修复、信任移交)。
- 去向面板 = 现 ChannelsPage(钉钉/企微渠道列表、添加/换凭据、测试发送)。
- 两个组件零改动复用,仅标题降级(见 §6)。

## 5. 设置页

内容不动,导航位置不变(侧栏底部)。

## 6. AppShell 与机制变更

- `PageId`:`"workbench" | "rules" | "integrations" | "settings"`。
- 默认页:`savedPage()` 默认值 `hooks` → `workbench`;localStorage 旧值映射:`overview/history → workbench`、`hooks/projects → rules`、`agents/channels → integrations`、`settings → settings`,未知值落 workbench。
- `openPage()` 不再带 `historySeed`,AppShell 回归纯导航。
- 侧栏仍为 184px rail + 48px header(健康点 + 待发/失败计数保留,属环境状态)。
- 被 embed 的 6 个页面组件 h1 → h2,保留各自 `<section aria-label>`;仅 4 个新页面有 h1,标题层级恢复正确。

## 7. 共享 TabBar 组件

新建 `src/shell/TabBar.tsx`(约 40 行):受控 tabs,button + `aria-selected` + `cc-focusable`,键盘左右切换。3 个页面共用,样式沿用现有 scope 单选观感。

## 8. i18n 变更

- 新增键:`navWorkbench`(工作台/Workbench)、`navRules`(通知规则/Rules)、`navIntegrations`(集成/Integrations),及 6 个 Tab 标签键。
- 旧键 `navOverview/navAgents/navHooks/navChannels/navProjects/navHistory` 保留(面板 aria-label 与页内文案仍在用),仅从导航数组移除。

## 9. 测试影响

| 文件 | 变更 |
|---|---|
| AppShell.test.tsx | 7 导航项断言 → 4;默认页断言 hooks → workbench;新增旧 localStorage 值迁移用例 |
| 各页面 *.test.tsx | 基本不动(组件独立渲染);如有 h1 断言同步降级 |
| 新增 WorkbenchPage.test.tsx | 失败预览切 Tab + 筛选生效;issue 跳转目标正确 |

## 10. 版本升级为 2.0.0(用户指示)

- `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 三处 `0.1.0 → 2.0.0`;`Cargo.lock` 由 cargo 构建自动跟进。
- 诊断导出的 `app_version` 取自 Cargo 包版本,自动生效,无需改动。

## 11. post-v1-issues.md → v2 待办文档(用户指示)

- 重命名 `docs/post-v1-issues.md` → `docs/v2-issues.md`,标题改为 v2 待办与遗留问题。
- 保留全部既有条目(仍有效);开头记录 v2 以主界面重组开场,并注明页内布局优化(HookRuleDrawer 重组、历史筛选器精简、视觉令牌)属 v2 后续条目。
- 仓库内无对该文件名的引用,无需改引用;记忆索引另行更新。

## 12. 明确不在本次范围(v2 后续轮次)

- HookRuleDrawer(930 行)页内结构重组
- HistoryPage 筛选器布局精简
- 视觉风格(颜色/间距/设计令牌)调整
- 任何后端/Rust 契约变更

## 13. 实施顺序

1. TabBar 组件 + 新 PageId/导航数组 + localStorage 迁移(AppShell)
2. 新建 3 个薄页面容器(Workbench/Rules/Integrations),embed 现有组件
3. OverviewPage `issuePage()`/`onNavigate` 目标改指向;h1→h2 降级
4. i18n 键 + 测试更新
5. 版本号三处升 2.0.0;post-v1-issues.md 重命名改造为 v2 文档

## 风险

- localStorage 旧值迁移(映射表处理)
- historySeed 下沉(工作台内部承接)
- 标题层级变化(h1→h2 降级,a11y 断言同步)
