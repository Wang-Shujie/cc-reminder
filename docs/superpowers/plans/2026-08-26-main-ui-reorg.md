# 主界面重组实施计划(7 页 → 4 页 + v2.0.0)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把主界面从 7 个平级页面重组为 4 个目的地(工作台/通知规则/集成/设置),页内用统一 TabBar 子导航;完成后版本升级 2.0.0、post-v1-issues.md 改造为 v2 文档。

**Architecture:** 现有 6 个页面组件(Overview/History/Agents/Channels/HookRules/Projects)零逻辑改动地嵌入 3 个新建薄容器页(AppShell 渲染 4 页);跳转机制从"跨页 seed"改为"页内 Tab 状态";localStorage 旧页 ID 读时映射。纯前端,后端契约零变更。

**Tech Stack:** React 19 + TypeScript strict + vitest/@testing-library + Playwright(含 axe-core 门禁与截图基线)+ Tauri 2。

**设计文档(spec):** `docs/superpowers/specs/2026-08-26-main-ui-reorg-design.md`

## Global Constraints

- 包管理器只用 pnpm;**不新增任何依赖**。
- zh 字典是权威,en 精确镜像(`Dictionary` 接口类型强制,`pnpm build` 的 tsc 会查)。
- a11y 基线:e2e axe serious/critical = 0;所有可交互控件带 `cc-focusable` 类。
- CSS 零新增:TabBar 复用现有 `.rules-tabs` / `.rules-tab` / `.rules-tab-active` 类(src/app.css:243-267)。
- 每个任务至少一次 git 提交(conventional commits,用户要求小步提交可回退);提交信息末尾带 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。
- 测试命令:`pnpm test`(vitest 全量)、单文件 `pnpm test -- src/shell/TabBar.test.tsx`、`pnpm test:e2e`(playwright)、`pnpm build`(tsc --noEmit + vite build)。
- 被嵌入的 6 个组件标题从 h1 降为 h2;只有 4 个新页面有 h1。

---

### Task 1: TabBar 共享组件

**Files:**
- Create: `src/shell/TabBar.tsx`
- Test: `src/shell/TabBar.test.tsx`

**Interfaces:**
- Consumes: 无(纯展示组件,复用 app.css 现有类名)
- Produces: `TabBar<T extends string>(props: { tabs: readonly { id: T; label: string }[]; active: T; onSelect: (id: T) => void; ariaLabel: string }): ReactNode` — Task 5 三个容器页消费

- [ ] **Step 1: 写失败测试**

```tsx
// src/shell/TabBar.test.tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState, type ReactNode } from "react";

import { TabBar } from "./TabBar";

const TABS = [
  { id: "alpha", label: "甲" },
  { id: "beta", label: "乙" },
] as const;

function Harness(): ReactNode {
  const [active, setActive] = useState<"alpha" | "beta">("alpha");
  return (
    <TabBar
      tabs={TABS}
      active={active}
      onSelect={setActive}
      ariaLabel="演示标签组"
    />
  );
}

test("renders a tablist with one selected tab", () => {
  render(<Harness />);
  expect(screen.getByRole("tablist", { name: "演示标签组" })).toBeVisible();
  expect(screen.getByRole("tab", { name: "甲" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(screen.getByRole("tab", { name: "乙" })).toHaveAttribute(
    "aria-selected",
    "false",
  );
});

test("click selects a tab", async () => {
  const user = userEvent.setup();
  render(<Harness />);
  await user.click(screen.getByRole("tab", { name: "乙" }));
  expect(screen.getByRole("tab", { name: "乙" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

test("arrow keys move selection with focus (automatic activation)", async () => {
  const user = userEvent.setup();
  render(<Harness />);
  const first = screen.getByRole("tab", { name: "甲" });
  await user.click(first);
  await user.keyboard("{ArrowRight}");
  expect(screen.getByRole("tab", { name: "乙" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(screen.getByRole("tab", { name: "乙" })).toHaveFocus();
  await user.keyboard("{ArrowLeft}");
  expect(screen.getByRole("tab", { name: "甲" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(screen.getByRole("tab", { name: "甲" })).toHaveFocus();
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm test -- src/shell/TabBar.test.tsx`
Expected: FAIL,无法解析 `./TabBar`

- [ ] **Step 3: 最小实现**

```tsx
// src/shell/TabBar.tsx
// Shared in-page tab strip for the 4-destination shell (spec §7). Reuses the
// rules-tabs visual language; deliberate simplification: no aria-controls /
// roving tabindex — every tab stays Tab-reachable, arrows auto-activate.
import { useRef, type KeyboardEvent, type ReactNode } from "react";

export interface TabItem<T extends string> {
  id: T;
  label: string;
}

export function TabBar<T extends string>({
  tabs,
  active,
  onSelect,
  ariaLabel,
}: {
  tabs: readonly TabItem<T>[];
  active: T;
  onSelect: (id: T) => void;
  ariaLabel: string;
}): ReactNode {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number): void {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
      return;
    }
    event.preventDefault();
    const delta = event.key === "ArrowLeft" ? -1 : 1;
    const next = (index + delta + tabs.length) % tabs.length;
    onSelect(tabs[next]!.id);
    buttons.current[next]?.focus();
  }

  return (
    <div role="tablist" aria-label={ariaLabel} className="rules-tabs">
      {tabs.map((tab, index) => (
        <button
          key={tab.id}
          ref={(el) => {
            buttons.current[index] = el;
          }}
          type="button"
          role="tab"
          aria-selected={active === tab.id}
          className={`cc-focusable rules-tab${active === tab.id ? " rules-tab-active" : ""}`}
          onClick={() => onSelect(tab.id)}
          onKeyDown={(event) => onKeyDown(event, index)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: 运行确认通过**

Run: `pnpm test -- src/shell/TabBar.test.tsx`
Expected: PASS(3 个测试)

- [ ] **Step 5: 提交**

```bash
git add src/shell/TabBar.tsx src/shell/TabBar.test.tsx
git commit -m "feat: shared TabBar component for in-page sub-navigation

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: i18n 新增字典键

**Files:**
- Modify: `src/lib/i18n.ts`(Dictionary 接口 + zhCn + en 三处)

**Interfaces:**
- Consumes: 无
- Produces: 12 个新键 `navWorkbench/navRules/navIntegrations/tabStatusOverview/tabNotificationLog/tabRuleTable/tabProjectManagement/tabSources/tabDestinations/gotoIntegrations/gotoRules/gotoHistoryTab` — Task 4/5/6 消费。旧的 `gotoAgents/gotoChannels/gotoHistory` 本任务**保留**(OverviewPage 仍在用,Task 4 才移除)。

- [ ] **Step 1: Dictionary 接口加键**

在 `interface Dictionary` 的 `navSettings: string;`(src/lib/i18n.ts:11)之后插入:

```ts
  navWorkbench: string;
  navRules: string;
  navIntegrations: string;
```

在接口内 `navLabel: string;` 之后插入:

```ts
  tabStatusOverview: string;
  tabNotificationLog: string;
  tabRuleTable: string;
  tabProjectManagement: string;
  tabSources: string;
  tabDestinations: string;
  gotoIntegrations: string;
  gotoRules: string;
  gotoHistoryTab: string;
```

- [ ] **Step 2: zhCn 加值**

在 `zhCn` 的 `navSettings: "设置",` 之后插入:

```ts
  navWorkbench: "工作台",
  navRules: "通知规则",
  navIntegrations: "集成",
```

在 `navLabel: "主导航",` 之后插入:

```ts
  tabStatusOverview: "状态概览",
  tabNotificationLog: "通知记录",
  tabRuleTable: "规则表",
  tabProjectManagement: "项目管理",
  tabSources: "通知来源",
  tabDestinations: "通知去向",
  gotoIntegrations: "前往集成",
  gotoRules: "前往规则",
  gotoHistoryTab: "查看通知记录",
```

- [ ] **Step 3: en 加值(精确镜像)**

在 `en` 的 `navSettings: "Settings",` 之后插入:

```ts
  navWorkbench: "Workbench",
  navRules: "Rules",
  navIntegrations: "Integrations",
```

在 `navLabel: "Navigation",` 之后插入:

```ts
  tabStatusOverview: "Status",
  tabNotificationLog: "Notification Log",
  tabRuleTable: "Rules",
  tabProjectManagement: "Projects",
  tabSources: "Sources",
  tabDestinations: "Destinations",
  gotoIntegrations: "Go to Integrations",
  gotoRules: "Go to Rules",
  gotoHistoryTab: "View notification log",
```

- [ ] **Step 4: 类型检查**

Run: `pnpm exec tsc --noEmit`
Expected: 无错误(两字典均补齐;`en: Dictionary` 类型强制镜像)

- [ ] **Step 5: 提交**

```bash
git add src/lib/i18n.ts
git commit -m "feat(i18n): nav/tab labels for the 4-destination shell

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: 六个内嵌组件 h1 → h2

**Files:**
- Modify: `src/overview/OverviewPage.tsx:127,143`、`src/agents/AgentsPage.tsx:203`、`src/channels/ChannelsPage.tsx:174`、`src/history/HistoryPage.tsx:270`、`src/hooks/HookRulesPage.tsx:218`、`src/projects/ProjectsPage.tsx:176`

**Interfaces:**
- Consumes: 无
- Produces: 六组件的页面标题均为 `<h2>`(为 Task 5 嵌入做准备;测试断言用不带 level 的 `heading` role,不受影响)

- [ ] **Step 1: 逐文件把标题行的 `<h1>` 改为 `<h2>`(共 7 处,OverviewPage 有 2 处:加载态与主态)**

每处形如 `<h1>{t.navOverview}</h1>` → `<h2>{t.navOverview}</h2>`。只改标题标签,不动其它内容。确认方式:

Run: `grep -n "<h1" src/overview/OverviewPage.tsx src/agents/AgentsPage.tsx src/channels/ChannelsPage.tsx src/history/HistoryPage.tsx src/hooks/HookRulesPage.tsx src/projects/ProjectsPage.tsx`
Expected: 无输出(六文件已无 h1;`src/settings/SettingsPage.tsx` 保留 h1 不动)

- [ ] **Step 2: 全量测试确认无破坏**

Run: `pnpm test`
Expected: PASS(页面测试的 heading 断言均不带 level;`AppShell.test` 的 `renderShell` 等 `level: 1` 标题由 AppShell 当前仍渲染的旧页面 h1 提供——注意:本任务后 AppShell 各页只有 h2,`renderShell` 的 `findByRole("heading", { level: 1 })` 将超时。**因此本任务与 Task 6 的 AppShell 测试更新须协调**:本步骤只运行**页面级测试**确认无破坏)

Run: `pnpm test -- src/overview src/agents src/channels src/history src/hooks src/projects`
Expected: PASS

说明:AppShell.test / App.test / Onboarding.test(断言 h1 标题)自本任务起**暂时红**,Task 6 统一修复——这是本计划唯一允许的跨任务红窗口;若执行者希望全绿提交,可将本任务与 Task 4、5 合并执行后一起跑全量。执行 subagent-driven 流程时,本任务结束后接 Task 4/5/6 连续执行,不要中途停顿等待评审之外的长时间间隔。

- [ ] **Step 3: 提交**

```bash
git add src/overview/OverviewPage.tsx src/agents/AgentsPage.tsx src/channels/ChannelsPage.tsx src/history/HistoryPage.tsx src/hooks/HookRulesPage.tsx src/projects/ProjectsPage.tsx
git commit -m "refactor: demote embedded page titles h1 -> h2

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: OverviewPage 跳转接口改造

**Files:**
- Modify: `src/overview/OverviewPage.tsx`
- Modify: `src/overview/OverviewPage.test.tsx`
- Modify: `src/lib/i18n.ts`(删除 3 个废弃 goto 键)

**Interfaces:**
- Consumes: Task 2 的 `gotoIntegrations/gotoRules/gotoHistoryTab`
- Produces: `OverviewPage(props: { locale?: LocaleCode; backend?: Backend; onNavigate?: (page: "rules" | "integrations") => void; onOpenHistory?: (deliveryStatus?: DeliveryStatusCode) => void })`。删除 `HistorySeed` 导出(Task 6 的 AppShell 不再引用);跨页跳转只允许 `"rules" | "integrations"`,历史下钻走 `onOpenHistory`。

- [ ] **Step 1: 更新测试(先行)**

`src/overview/OverviewPage.test.tsx` 四处修改:

(a) "missing agent, drift and trust-pending issues navigate to Agent integrations" 测试:按钮名 `"前往 Agent 集成"` → `"前往集成"`,断言 `expect(onNavigate).toHaveBeenCalledWith("agents")` → `toHaveBeenCalledWith("integrations")`,并新增 `const onOpenHistory = vi.fn();` 传入组件。

(b) "unavailable credential store and paused channel issues navigate to Channels" 测试:同样把按钮名 `"前往渠道"` → `"前往集成"`,断言改为 `toHaveBeenCalledWith("integrations")`,传入 onOpenHistory。

(c) "查看失败任务 navigates to history pre-filtered to failed jobs" 测试改为:

```tsx
test("查看失败任务 opens the notification log pre-filtered to failed jobs", async () => {
  const onOpenHistory = vi.fn();
  const user = userEvent.setup();
  render(
    <OverviewPage backend={configuredBackend()} onOpenHistory={onOpenHistory} />,
  );
  await user.click(await screen.findByRole("button", { name: "查看失败任务" }));
  expect(onOpenHistory).toHaveBeenCalledWith("failed");
});
```

(d) 新增队列类 issue 下钻测试(插在渠道测试之后):

```tsx
test("queue and delivery issues open the notification log tab", async () => {
  const onNavigate = vi.fn();
  const onOpenHistory = vi.fn();
  const user = userEvent.setup();
  render(
    <OverviewPage
      backend={healthBackend({
        issues: [
          {
            issue_code: "delivery.repeated_failure",
            level: "error",
            message: "渠道连续失败",
            suggested_command: null,
            suggested_action: null,
          },
        ],
      })}
      onNavigate={onNavigate}
      onOpenHistory={onOpenHistory}
    />,
  );
  const button = await screen.findByRole("button", { name: "查看通知记录" });
  await user.click(button);
  expect(onOpenHistory).toHaveBeenCalled();
  expect(onNavigate).not.toHaveBeenCalled();
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm test -- src/overview/OverviewPage.test.tsx`
Expected: FAIL(按钮名/回调签名不匹配)

- [ ] **Step 3: 实现**

`src/overview/OverviewPage.tsx` 修改:

(a) imports:删 `HistorySeed` 相关(见 e);`PageId` 导入改为删除(不再使用);新增 `DeliveryStatusCode` 已在现有 import 中(确认保留)。

(b) 删除旧 `issuePage` 与 `HistorySeed`,替换为:

```ts
/** Where an issue's repair lives after the 4-destination reorg. */
type IssueAction =
  | { kind: "page"; page: "rules" | "integrations" }
  | { kind: "history" };

function issueAction(issueCode: string): IssueAction {
  if (
    issueCode.startsWith("queue.") ||
    issueCode.startsWith("delivery.") ||
    issueCode.startsWith("spool")
  ) {
    return { kind: "history" };
  }
  if (issueCode.startsWith("hooks.") || issueCode.startsWith("projects.")) {
    return { kind: "page", page: "rules" };
  }
  return { kind: "page", page: "integrations" };
}
```

(c) 组件 props 与回调:

```tsx
export function OverviewPage({
  locale = "zh_cn",
  backend: injected,
  onNavigate,
  onOpenHistory,
}: {
  locale?: LocaleCode;
  backend?: Backend;
  onNavigate?: (page: "rules" | "integrations") => void;
  onOpenHistory?: (deliveryStatus?: DeliveryStatusCode) => void;
}): ReactNode {
```

(d) 渲染:指标条上 `查看失败任务` 按钮的 onClick 改为 `() => onOpenHistory?.("failed")`;issue 列表的 `navLabelFor` 辅助函数替换为:

```tsx
const actionLabel = (action: IssueAction): string =>
  action.kind === "history"
    ? t.gotoHistoryTab
    : action.page === "rules"
      ? t.gotoRules
      : t.gotoIntegrations;
```

map 回调内原有的 `const target = issuePage(issue.issue_code);` 改为 `const action = issueAction(issue.issue_code);`,按钮(label 与 onClick)替换为:

```tsx
                  <button
                    type="button"
                    className="cc-focusable"
                    onClick={() =>
                      action.kind === "history"
                        ? onOpenHistory?.()
                        : onNavigate?.(action.page)
                    }
                  >
                    {actionLabel(action)}
                  </button>
```

(保留原有的 issue level 圆点、message、suggested_command/action 渲染不变;只有按钮部分按上面替换。)

(e) 删除 `export interface HistorySeed {...}` 块。

- [ ] **Step 4: 移除废弃 goto 键**

先确认唯一使用方已切换:

Run: `grep -rn "gotoAgents\|gotoChannels\|gotoHistory\b" src --include="*.ts" --include="*.tsx"`
Expected: 仅 `src/lib/i18n.ts` 命中(接口 + zhCn + en 各 3 行)

然后从 `Dictionary` 接口、`zhCn`、`en` 三处删除 `gotoAgents/gotoChannels/gotoHistory` 共 9 行。

- [ ] **Step 5: 运行确认通过**

Run: `pnpm test -- src/overview/OverviewPage.test.tsx && pnpm exec tsc --noEmit`
Expected: PASS / 无错误(tsc 会抓住任何残留的旧键引用,包括尚未改造的 AppShell —— **注意**:AppShell 仍引用 `HistorySeed` 类型,本任务 Step 3(e) 删除导出会让 tsc 报错。处理:同一步把 `src/shell/AppShell.tsx` 第 19 行的 `import { OverviewPage, type HistorySeed }` 改为 `import { OverviewPage }`,并删掉 `historySeed` state、`openPage` 的 seed 参数与向 HistoryPage 传 `initialDeliveryStatus` 的三元(该页签将在 Task 6 重写,此处最小改动保持编译绿:HistoryPage 直接无 prop 渲染)。vitest 的 AppShell 相关测试仍会红(标题断言),留给 Task 6。)

Run: `pnpm test -- src/overview`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add src/overview/OverviewPage.tsx src/overview/OverviewPage.test.tsx src/lib/i18n.ts src/shell/AppShell.tsx
git commit -m "refactor(overview): in-page history drilldown + rules/integrations jump targets

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: 三个薄容器页(工作台 / 通知规则 / 集成)

**Files:**
- Create: `src/workbench/WorkbenchPage.tsx` + `src/workbench/WorkbenchPage.test.tsx`
- Create: `src/rules/RulesPage.tsx` + `src/rules/RulesPage.test.tsx`
- Create: `src/integrations/IntegrationsPage.tsx` + `src/integrations/IntegrationsPage.test.tsx`

**Interfaces:**
- Consumes: Task 1 `TabBar`;Task 4 `OverviewPage(onNavigate, onOpenHistory)`;现有 `HistoryPage/HookRulesPage/ProjectsPage/AgentsPage/ChannelsPage`
- Produces: `WorkbenchPage({ locale, onNavigate, backend? })`、`RulesPage({ locale, backend? })`、`IntegrationsPage({ locale, backend? })`(backend 可选注入,测试用;实现里透传给子页)。`HistoryPage` 的 `key={filter}` 重挂载模式由 WorkbenchPage 拥有。

- [ ] **Step 1: 写失败测试(三个文件)**

```tsx
// src/workbench/WorkbenchPage.test.tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { BackendProvider } from "../lib/backend";
import { configuredBackend, type FakeBackend } from "../test/TestApp";
import type { ChannelId, HistoryItem } from "../lib/contracts";
import { WorkbenchPage } from "./WorkbenchPage";

function item(overrides: Partial<HistoryItem>): HistoryItem {
  return {
    event_id: "evt-1",
    source: "claude-code",
    source_version: "2.1.218",
    source_event: "PostToolUseFailure",
    category: "failure",
    occurred_at: "2026-08-20T10:00:00+08:00",
    received_at: "2026-08-20T10:00:01+08:00",
    project_id: null,
    project_display_name: null,
    unmatched_cwd_fingerprint: "sha256:9f2c",
    model: null,
    permission_mode: null,
    severity: "error",
    public_fields: {},
    correlation_id: "corr-1",
    processing_outcome: "queued",
    outcome_reason_code: null,
    delivery_job_id: "job-1",
    channel_id: "ch-1" as ChannelId,
    document: null,
    delivery_status: "failed",
    attempts: [],
    ...overrides,
  };
}

function renderWorkbench(backend: FakeBackend) {
  render(
    <BackendProvider backend={backend}>
      <WorkbenchPage locale="zh_cn" />
    </BackendProvider>,
  );
}

test("defaults to the status overview tab", async () => {
  renderWorkbench(configuredBackend());
  expect(
    await screen.findByRole("heading", { name: "工作台", level: 1 }),
  ).toBeVisible();
  expect(screen.getByRole("tab", { name: "状态概览" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(screen.getByRole("heading", { name: "概览" })).toBeVisible();
});

test("查看失败任务 switches to the log tab pre-filtered to failed", async () => {
  const user = userEvent.setup();
  renderWorkbench(
    configuredBackend({
      historyItems: [
        item({}),
        item({
          event_id: "evt-2",
          source_event: "StopSuccess",
          delivery_status: "delivered",
          delivery_job_id: "job-2",
        }),
      ],
    }),
  );
  await user.click(await screen.findByRole("button", { name: "查看失败任务" }));
  expect(screen.getByRole("tab", { name: "通知记录" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(await screen.findByText("PostToolUseFailure")).toBeVisible();
  expect(screen.queryByText("StopSuccess")).not.toBeInTheDocument();
});

test("switching back to 状态概览 restores the overview panel", async () => {
  const user = userEvent.setup();
  renderWorkbench(configuredBackend());
  await user.click(await screen.findByRole("button", { name: "查看失败任务" }));
  await user.click(screen.getByRole("tab", { name: "状态概览" }));
  expect(screen.getByRole("heading", { name: "概览" })).toBeVisible();
});
```

```tsx
// src/rules/RulesPage.test.tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { BackendProvider } from "../lib/backend";
import { configuredBackend } from "../test/TestApp";
import { RulesPage } from "./RulesPage";

test("defaults to the rules tab and can switch to project management", async () => {
  const user = userEvent.setup();
  render(
    <BackendProvider backend={configuredBackend()}>
      <RulesPage locale="zh_cn" />
    </BackendProvider>,
  );
  expect(
    await screen.findByRole("heading", { name: "通知规则", level: 1 }),
  ).toBeVisible();
  expect(screen.getByRole("heading", { name: "Hook 规则" })).toBeVisible();
  await user.click(screen.getByRole("tab", { name: "项目管理" }));
  expect(screen.getByRole("heading", { name: "项目" })).toBeVisible();
});
```

```tsx
// src/integrations/IntegrationsPage.test.tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { BackendProvider } from "../lib/backend";
import { configuredBackend } from "../test/TestApp";
import { IntegrationsPage } from "./IntegrationsPage";

test("defaults to sources and can switch to destinations", async () => {
  const user = userEvent.setup();
  render(
    <BackendProvider backend={configuredBackend()}>
      <IntegrationsPage locale="zh_cn" />
    </BackendProvider>,
  );
  expect(
    await screen.findByRole("heading", { name: "集成", level: 1 }),
  ).toBeVisible();
  expect(screen.getByRole("heading", { name: "Agent 集成" })).toBeVisible();
  await user.click(screen.getByRole("tab", { name: "通知去向" }));
  expect(screen.getByRole("heading", { name: "渠道" })).toBeVisible();
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm test -- src/workbench src/rules src/integrations`
Expected: FAIL,三个模块无法解析

- [ ] **Step 3: 实现三个容器**

```tsx
// src/workbench/WorkbenchPage.tsx
// 工作台:状态概览 + 通知记录两个页签。历史下钻(seed 下沉)在此承接:
// OverviewPage 的 onOpenHistory 切页签并作为 HistoryPage 的初始筛选,
// key 变化强制重挂载使筛选立即生效。页签选择不持久化——进页回到概览。
import { useState, type ReactNode } from "react";

import type { Backend } from "../lib/backend";
import type { DeliveryStatusCode, LocaleCode } from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { HistoryPage } from "../history/HistoryPage";
import { OverviewPage } from "../overview/OverviewPage";
import { TabBar } from "../shell/TabBar";
import type { PageId } from "../shell/AppShell";

type WorkbenchTab = "overview" | "history";

export function WorkbenchPage({
  locale,
  backend: injected,
  onNavigate,
}: {
  locale: LocaleCode;
  backend?: Backend;
  onNavigate?: (page: PageId) => void;
}): ReactNode {
  const t = dictionary(locale);
  const [tab, setTab] = useState<WorkbenchTab>("overview");
  const [historyFilter, setHistoryFilter] = useState<DeliveryStatusCode | null>(
    null,
  );

  function openHistory(deliveryStatus?: DeliveryStatusCode): void {
    setHistoryFilter(deliveryStatus ?? null);
    setTab("history");
  }

  return (
    <section aria-label={t.navWorkbench}>
      <h1>{t.navWorkbench}</h1>
      <TabBar
        ariaLabel={t.navWorkbench}
        active={tab}
        onSelect={setTab}
        tabs={[
          { id: "overview", label: t.tabStatusOverview },
          { id: "history", label: t.tabNotificationLog },
        ]}
      />
      {tab === "overview" ? (
        <OverviewPage
          locale={locale}
          backend={injected}
          onNavigate={onNavigate}
          onOpenHistory={openHistory}
        />
      ) : (
        <HistoryPage
          key={historyFilter ?? "all"}
          locale={locale}
          backend={injected}
          initialDeliveryStatus={historyFilter}
        />
      )}
    </section>
  );
}
```

说明:容器自身不取 backend——两个子页各自经 `usePageBackend(injected)` 解析(注入优先、否则 context),容器只透传。

```tsx
// src/rules/RulesPage.tsx
// 通知规则:规则表 + 项目管理两个页签。两个子页状态机完全隔离
// (设计 §3):规则表自带全局/项目 scope 单选,项目管理独立增删根目录。
import { useState, type ReactNode } from "react";

import type { Backend } from "../lib/backend";
import type { LocaleCode } from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { HookRulesPage } from "../hooks/HookRulesPage";
import { ProjectsPage } from "../projects/ProjectsPage";
import { TabBar } from "../shell/TabBar";

type RulesTab = "rules" | "projects";

export function RulesPage({
  locale,
  backend: injected,
}: {
  locale: LocaleCode;
  backend?: Backend;
}): ReactNode {
  const t = dictionary(locale);
  const [tab, setTab] = useState<RulesTab>("rules");
  return (
    <section aria-label={t.navRules}>
      <h1>{t.navRules}</h1>
      <TabBar
        ariaLabel={t.navRules}
        active={tab}
        onSelect={setTab}
        tabs={[
          { id: "rules", label: t.tabRuleTable },
          { id: "projects", label: t.tabProjectManagement },
        ]}
      />
      {tab === "rules" ? (
        <HookRulesPage locale={locale} />
      ) : (
        <ProjectsPage locale={locale} backend={injected} />
      )}
    </section>
  );
}
```

说明:`HookRulesPage` 只从 context 取 backend(无注入 prop),测试经 `BackendProvider` 提供即可。

```tsx
// src/integrations/IntegrationsPage.tsx
// 集成:通知来源(Agent)与通知去向(渠道)——同一条流水线的两端。
import { useState, type ReactNode } from "react";

import type { Backend } from "../lib/backend";
import type { LocaleCode } from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { AgentsPage } from "../agents/AgentsPage";
import { ChannelsPage } from "../channels/ChannelsPage";
import { TabBar } from "../shell/TabBar";

type IntegrationsTab = "sources" | "destinations";

export function IntegrationsPage({
  locale,
  backend: injected,
}: {
  locale: LocaleCode;
  backend?: Backend;
}): ReactNode {
  const t = dictionary(locale);
  const [tab, setTab] = useState<IntegrationsTab>("sources");
  return (
    <section aria-label={t.navIntegrations}>
      <h1>{t.navIntegrations}</h1>
      <TabBar
        ariaLabel={t.navIntegrations}
        active={tab}
        onSelect={setTab}
        tabs={[
          { id: "sources", label: t.tabSources },
          { id: "destinations", label: t.tabDestinations },
        ]}
      />
      {tab === "sources" ? (
        <AgentsPage locale={locale} backend={injected} />
      ) : (
        <ChannelsPage locale={locale} backend={injected} />
      )}
    </section>
  );
}
```

- [ ] **Step 4: 运行确认通过**

Run: `pnpm test -- src/workbench src/rules src/integrations`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/workbench src/rules src/integrations
git commit -m "feat: workbench/rules/integrations tabbed container pages

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: AppShell 切换四页 + localStorage 迁移 + 壳层测试修复

**Files:**
- Modify: `src/shell/AppShell.tsx`(全量重写,见下)
- Modify: `src/shell/AppShell.test.tsx`(全量重写)
- Modify: `src/App.test.tsx:11-16`
- Modify: `src/onboarding/Onboarding.test.tsx:69`

**Interfaces:**
- Consumes: Task 5 三容器;Task 2 i18n 键
- Produces: `PageId = "workbench" | "rules" | "integrations" | "settings"`(WorkbenchPage 已 `import type` 消费);启动默认页 workbench;旧页 ID 读时映射

- [ ] **Step 1: 重写 AppShell 测试(先行)**

`src/shell/AppShell.test.tsx` 全量替换为:

```tsx
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { TestApp, configuredBackend, type FakeBackend } from "../test/TestApp";

// vitest's jsdom rewrites import.meta.url to http://localhost — resolve the
// stylesheet from the project root instead.
const appCss = readFileSync(resolve(process.cwd(), "src/app.css"), "utf8");

async function renderShell(backend: FakeBackend) {
  render(<TestApp backend={backend} />);
  // Locale-independent: the zh and en shells use different headings.
  await screen.findByRole("heading", { level: 1, name: "工作台" });
}

test("navigation is keyboard accessible and remembers the selected page", async () => {
  const user = userEvent.setup();
  const backend = configuredBackend();
  await renderShell(backend);
  await user.click(screen.getByRole("button", { name: "通知规则" }));
  expect(
    screen.getByRole("heading", { name: "通知规则", level: 1 }),
  ).toBeVisible();
  expect(localStorage.getItem("cc-reminder:last-page")).toBe("rules");
  expect(screen.getByRole("button", { name: "通知规则" })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("selected page survives a remount via localStorage", async () => {
  const user = userEvent.setup();
  const { unmount } = render(<TestApp backend={configuredBackend()} />);
  await screen.findByRole("heading", { name: "工作台" });
  await user.click(screen.getByRole("button", { name: "设置" }));
  expect(screen.getByRole("heading", { name: "设置", level: 1 })).toBeVisible();
  unmount();
  render(<TestApp backend={configuredBackend()} />);
  expect(await screen.findByRole("heading", { name: "设置" })).toBeVisible();
});

test("legacy v1 page ids migrate to the new destinations", async () => {
  for (const [legacy, heading] of [
    ["overview", "工作台"],
    ["history", "工作台"],
    ["hooks", "通知规则"],
    ["projects", "通知规则"],
    ["agents", "集成"],
    ["channels", "集成"],
    ["settings", "设置"],
  ] as const) {
    localStorage.setItem("cc-reminder:last-page", legacy);
    const { unmount } = render(<TestApp backend={configuredBackend()} />);
    expect(
      await screen.findByRole("heading", { name: heading, level: 1 }),
    ).toBeVisible();
    unmount();
  }
});

test("unknown legacy value falls back to the workbench", async () => {
  localStorage.setItem("cc-reminder:last-page", "nonsense");
  render(<TestApp backend={configuredBackend()} />);
  expect(
    await screen.findByRole("heading", { name: "工作台", level: 1 }),
  ).toBeVisible();
});

test("core revision events refresh health instead of trusting payload data", async () => {
  const backend = configuredBackend();
  render(<TestApp backend={backend} />);
  await screen.findByRole("heading", { name: "工作台" });
  expect(backend.getHealthSnapshot).toHaveBeenCalledTimes(1);
  act(() => {
    backend.emit("core://health-changed", { revision: 4, overall: "forged" });
  });
  await waitFor(() => expect(backend.getHealthSnapshot).toHaveBeenCalledTimes(2));
  expect(screen.queryByText("forged")).not.toBeInTheDocument();
});

test("queue revision events also trigger a refetch", async () => {
  const backend = configuredBackend();
  render(<TestApp backend={backend} />);
  await screen.findByRole("heading", { name: "工作台" });
  act(() => {
    backend.emit("core://queue-changed", { revision: 7 });
  });
  await waitFor(() => expect(backend.getHealthSnapshot).toHaveBeenCalledTimes(2));
});

test("labels default to Chinese", async () => {
  await renderShell(configuredBackend());
  expect(screen.getByRole("button", { name: "工作台" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Workbench" })).not.toBeInTheDocument();
});

test("locale can follow the saved setting (English)", async () => {
  await renderShell(configuredBackend({ locale: "en" }));
  expect(screen.getByRole("button", { name: "Workbench" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Rules" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "工作台" })).not.toBeInTheDocument();
});

test("theme follows the saved setting; system resolves to the system attribute", async () => {
  await renderShell(configuredBackend({ theme: "system" }));
  expect(document.documentElement.dataset.theme).toBe("system");
});

test("explicit dark theme is applied verbatim for CSS to consume", async () => {
  await renderShell(configuredBackend({ theme: "dark" }));
  expect(document.documentElement.dataset.theme).toBe("dark");
});

test("focus is visible on navigation controls", async () => {
  const user = userEvent.setup();
  await renderShell(configuredBackend());
  await user.tab();
  expect(screen.getByRole("button", { name: "工作台" })).toHaveFocus();
  expect(screen.getByRole("button", { name: "工作台" })).toHaveClass("cc-focusable");
  // The stylesheet must define the visible focus treatment.
  expect(appCss).toContain(":focus-visible");
  expect(appCss).toContain("outline");
});

test("all four navigation targets are present", async () => {
  await renderShell(configuredBackend());
  for (const label of ["工作台", "通知规则", "集成", "设置"]) {
    expect(screen.getByRole("button", { name: label })).toBeVisible();
  }
});

test("configured startup defaults to the workbench", async () => {
  await renderShell(configuredBackend());
  expect(screen.getByRole("button", { name: "工作台" })).toHaveAttribute(
    "aria-current",
    "page",
  );
  // The default is not persisted as if the user had chosen it.
  expect(localStorage.getItem("cc-reminder:last-page")).toBeNull();
});

test("960 x 640 fixture: rail, header, content stay structurally separate", async () => {
  window.innerWidth = 960;
  window.innerHeight = 640;
  try {
    await renderShell(configuredBackend());
    const shell = document.querySelector(".shell-root");
    const header = document.querySelector(".shell-header");
    const nav = document.querySelector(".shell-nav");
    const content = document.querySelector(".shell-content");
    expect(shell).not.toBeNull();
    expect(header).not.toBeNull();
    expect(nav).not.toBeNull();
    expect(content).not.toBeNull();
    // Non-overlap contract: one grid root owns three disjoint regions.
    expect(header!.parentElement).toBe(shell);
    expect(nav!.parentElement).toBe(shell);
    expect(content!.parentElement).toBe(shell);
    expect(nav!.contains(header!)).toBe(false);
    expect(header!.contains(content!)).toBe(false);
    // Fixed rail width / header height / minimum window come from CSS
    // (jsdom performs no layout, so the geometry lives in the stylesheet).
    expect(appCss).toContain("grid-template-columns: 184px 1fr");
    expect(appCss).toMatch(/grid-template-rows:\s*48px 1fr/);
    expect(appCss).toMatch(/min-width:\s*960px/);
    expect(appCss).toMatch(/min-height:\s*640px/);
    // Quiet aesthetic: no viewport-scaled fonts, letter spacing pinned to 0,
    // radii capped at 8px, health-only color accents.
    expect(appCss).not.toMatch(/\d+vmin|\d+vw/);
    expect(appCss).toContain("letter-spacing: 0");
    for (const radius of appCss.matchAll(/border-radius:\s*([^;]+);/g)) {
      const values = radius[1]?.match(/\d+/g) ?? [];
      for (const v of values) {
        expect(Number(v)).toBeLessThanOrEqual(8);
      }
    }
  } finally {
    window.innerWidth = 1024;
    window.innerHeight = 768;
  }
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm test -- src/shell/AppShell.test.tsx`
Expected: FAIL(仍是 7 页旧壳)

- [ ] **Step 3: 重写 AppShell.tsx**

全量替换 `src/shell/AppShell.tsx` 为:

```tsx
// Quiet desktop shell: 184px rail + 48px header + unframed content, one grid.
// Four destinations (spec §1); in-page sub-navigation lives in each page's
// TabBar. Health colors are the only accents; everything else is neutral.
import { useEffect, useState, type ReactNode } from "react";
import {
  LayoutDashboard,
  ListChecks,
  Settings as SettingsIcon,
  Webhook,
} from "lucide-react";

import { useBackend } from "../lib/backend";
import { IntegrationsPage } from "../integrations/IntegrationsPage";
import { RulesPage } from "../rules/RulesPage";
import { SettingsPage } from "../settings/SettingsPage";
import { WorkbenchPage } from "../workbench/WorkbenchPage";
import {
  CORE_EVENTS,
  type HealthSnapshot,
  type LocaleCode,
  type ThemeCode,
} from "../lib/contracts";
import { dictionary, type Dictionary } from "../lib/i18n";

export type PageId = "workbench" | "rules" | "integrations" | "settings";

const PAGES: readonly {
  id: PageId;
  icon: typeof Webhook;
  label: (d: Dictionary) => string;
}[] = [
  { id: "workbench", icon: LayoutDashboard, label: (d) => d.navWorkbench },
  { id: "rules", icon: ListChecks, label: (d) => d.navRules },
  { id: "integrations", icon: Webhook, label: (d) => d.navIntegrations },
  { id: "settings", icon: SettingsIcon, label: (d) => d.navSettings },
];

/** v1 页 ID → v2 目的地(读时映射,写时永远写新 ID)。 */
const LEGACY_PAGE_MAP: Record<string, PageId> = {
  overview: "workbench",
  history: "workbench",
  hooks: "rules",
  projects: "rules",
  agents: "integrations",
  channels: "integrations",
  settings: "settings",
};

const LAST_PAGE_KEY = "cc-reminder:last-page";

function savedPage(): PageId {
  const value = localStorage.getItem(LAST_PAGE_KEY);
  if (value !== null && value in LEGACY_PAGE_MAP) {
    return LEGACY_PAGE_MAP[value]!;
  }
  return PAGES.some((page) => page.id === value) ? (value as PageId) : "workbench";
}

export function AppShell({
  locale,
  theme,
}: {
  locale: LocaleCode;
  theme: ThemeCode;
}): ReactNode {
  const backend = useBackend();
  const t = dictionary(locale);
  const [page, setPage] = useState<PageId>(savedPage);
  const [health, setHealth] = useState<HealthSnapshot | null>(null);

  // One snapshot on mount; revision events trigger a refetch. Event payloads
  // are never trusted for state — only the revision number arrives here.
  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      backend
        .getHealthSnapshot()
        .then((snapshot) => {
          if (!cancelled) {
            setHealth(snapshot);
          }
        })
        .catch(() => {
          /* offline in tests / transient core error: keep last snapshot */
        });
    };
    refresh();
    const subscriptions = CORE_EVENTS.map((event) =>
      backend.subscribe(event, () => {
        refresh();
      }),
    );
    return () => {
      cancelled = true;
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  }, [backend]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  function openPage(id: PageId): void {
    setPage(id);
    localStorage.setItem(LAST_PAGE_KEY, id);
  }

  const overall = health?.overall ?? "ok";

  return (
    <div className="shell-root" data-overall={overall}>
      <header className="shell-header">
        <span className="shell-title">{t.statusTitle}</span>
        <span className={`health-dot health-${overall}`} aria-hidden="true" />
        <span className="shell-counts">
          {t.pendingJobs}: {health?.pending_jobs ?? 0} · {t.failedJobs}:{" "}
          {health?.failed_jobs ?? 0}
        </span>
      </header>
      <nav className="shell-nav" aria-label={t.navLabel}>
        {PAGES.map(({ id, icon: Icon, label }) => (
          <button
            key={id}
            type="button"
            className={`nav-item cc-focusable${page === id ? " nav-active" : ""}`}
            aria-current={page === id ? "page" : undefined}
            onClick={() => openPage(id)}
          >
            <Icon size={16} aria-hidden="true" />
            <span>{label(t)}</span>
          </button>
        ))}
      </nav>
      <main className="shell-content">
        {page === "workbench" ? (
          <WorkbenchPage locale={locale} onNavigate={openPage} />
        ) : page === "rules" ? (
          <RulesPage locale={locale} />
        ) : page === "integrations" ? (
          <IntegrationsPage locale={locale} />
        ) : (
          <SettingsPage locale={locale} />
        )}
      </main>
    </div>
  );
}
```

- [ ] **Step 4: 修 App.test 与 Onboarding.test 的落点断言**

`src/App.test.tsx:11-16` 改为:

```tsx
test("renders the app shell at the saved page when bootstrap is complete", async () => {
  // v1 的 "channels" 读时迁移到 v2 的 "integrations"。
  localStorage.setItem("cc-reminder:last-page", "channels");
  render(<TestApp backend={configuredBackend()} />);
  expect(await screen.findByRole("heading", { name: "集成" })).toBeVisible();
  localStorage.removeItem("cc-reminder:last-page");
});
```

`src/onboarding/Onboarding.test.tsx:69`:`"Hook 规则"` → `"工作台"`(引导完成落新默认页)。

- [ ] **Step 5: 全量单测 + 类型检查**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: 全部 PASS / 无错误

- [ ] **Step 6: 提交**

```bash
git add src/shell/AppShell.tsx src/shell/AppShell.test.tsx src/App.test.tsx src/onboarding/Onboarding.test.tsx
git commit -m "feat(shell): 4-destination nav with legacy page-id migration

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: e2e 重写 + 截图基线重生成 + 文档图导出器

**Files:**
- Modify: `tests/e2e/app.spec.ts`
- Modify: `tests/e2e/export-doc-image.spec.ts`
- Delete/Regen: `tests/e2e/app.spec.ts-snapshots/`(整目录)
- Regen: `docs/images/hook-rules.png`

**Interfaces:**
- Consumes: Task 5/6 的页面结构(tab role、四导航)
- Produces: 绿色的 e2e 套件与新截图基线

- [ ] **Step 1: 更新 app.spec.ts**

(a) NAV_PAGES/SETTLE_TARGETS/openTab(替换第 20-54 行区域):

```ts
/** Nav rail labels (zh-CN authoritative dictionary) + snapshot stems. */
const NAV_PAGES = [
  ["工作台", "workbench"],
  ["通知规则", "rules"],
  ["集成", "integrations"],
  ["设置", "settings"],
] as const;

const SETTLE_TARGETS: Record<(typeof NAV_PAGES)[number][1], () => Promise<unknown>> = {
  workbench: (page) => page.getByRole("heading", { name: "待处理问题" }).waitFor(),
  rules: (page) => page.getByRole("row", { name: /Stop/ }).waitFor(),
  integrations: (page) => page.getByRole("row", { name: /Claude Code/ }).first().waitFor(),
  // Hydration enables the retention inputs; value proves get_settings landed.
  settings: (page) => expect(page.locator("#settings-event-days")).toHaveValue("30"),
};

async function openPage(page: Page, label: string): Promise<void> {
  await page.getByRole("button", { name: label }).click();
}

/** In-page TabBar tab (role=tab), e.g. 通知去向 inside 集成. */
async function openTab(page: Page, label: string): Promise<void> {
  await page.getByRole("tab", { name: label }).click();
}
```

(b) "configures a project override" 测试:
- 第 141 行 `openPage(page, "Hook 规则")` → `openPage(page, "通知规则")`
- 第 168-169 行渠道跳转改为:

```ts
    await openPage(page, "集成");
    await openTab(page, "通知去向");
    await page.getByRole("cell", { name: "值班群", exact: true }).waitFor();
```

(c) onboarding 完成测试(第 212-213 行):

```ts
    await page.getByRole("heading", { name: "工作台" }).waitFor();
    await expect(page.getByRole("button", { name: "通知规则" })).toBeVisible();
```

(d) "keyboard-only rule edit" 测试(第 217-218 行之间插入导航):

```ts
    await page.goto("/");
    await openPage(page, "通知规则");
    await page.getByRole("row", { name: /Stop/ }).waitFor();
```

(e) "all primary pages fit" 测试第 244 行:`getByRole("row", { name: /Stop/ })` 初次等待 → `page.getByRole("heading", { name: "待处理问题" }).waitFor(); // default workbench ready`

(f) "hook rules fit at 1280×800 and 1440×900" 测试:goto 后加 `await openPage(page, "通知规则");`,两处截图名 `hooks-1280x800.png`/`hooks-1440x900.png` → `rules-1280x800.png`/`rules-1440x900.png`。

(g) 200% zoom 测试第 269 行:同 (e) 改为等待工作台标题。

(h) light/dark 测试:`openPage(page, "Hook 规则")` → `openPage(page, "通知规则")`,截图 `hooks-dark.png` → `rules-dark.png`。

(i) English 测试:最长导航标签从 "Notification History" 改为 "Integrations";goto 后加 `await openPage(page, "Rules");` 再等 Stop 行;截图 `hooks-en.png` → `rules-en.png`:

```ts
    const longest = page.getByRole("button", { name: "Integrations" });
    await longest.waitFor();
    await openPage(page, "Rules");
    await page.getByRole("row", { name: /Stop/ }).waitFor();
```

- [ ] **Step 2: 更新 export-doc-image.spec.ts**

`page.goto("/")` 之后、等 Stop 行之前插入:

```ts
  await page.getByRole("button", { name: "通知规则" }).click();
```

- [ ] **Step 3: 重生成截图基线**

```bash
rm -rf tests/e2e/app.spec.ts-snapshots
pnpm test:e2e -- --update-snapshots
```

Expected: 套件通过并生成新基线(workbench/rules/integrations/settings + rules-1280x800/rules-1440x900/rules-dark/rules-en,均为 `-chromium-darwin` 后缀)

- [ ] **Step 4: 干净全量跑一遍确认基线稳定**

Run: `pnpm test:e2e`
Expected: PASS(基线已固定,无 diff)

- [ ] **Step 5: 重导 README 文档图**

```bash
CC_REMINDER_EXPORT_DOCS=1 pnpm test:e2e --export-doc-image
```

Expected: `docs/images/hook-rules.png` 更新为通知规则页新截图(README 引用该图,内容保持真实)

- [ ] **Step 6: 提交**

```bash
git add tests/e2e docs/images/hook-rules.png
git commit -m "test(e2e): 4-destination flows, regenerated baselines and doc image

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: 版本升级 2.0.0

**Files:**
- Modify: `package.json:3`、`src-tauri/tauri.conf.json:4`、`src-tauri/Cargo.toml:3`
- Regen: `src-tauri/Cargo.lock`(cc-reminder 条目)

**Interfaces:**
- Consumes: 无
- Produces: 三处版本 `2.0.0`;诊断导出 `app_version` 随 Cargo 自动变为 2.0.0

- [ ] **Step 1: 三处版本号 0.1.0 → 2.0.0**

`package.json` 第 3 行、`src-tauri/tauri.conf.json` 第 4 行 `"version": "0.1.0"` → `"2.0.0"`、`src-tauri/Cargo.toml` 第 3 行 → `version = "2.0.0"`。

- [ ] **Step 2: 更新 Cargo.lock**

Run: `cd src-tauri && cargo update -p cc-reminder && cd ..`
Expected: `Cargo.lock` 中 `name = "cc-reminder"` 的 version 变为 2.0.0

验证:`grep -A1 '^name = "cc-reminder"' src-tauri/Cargo.lock`

- [ ] **Step 3: 构建 + 全量验证**

Run: `pnpm build && pnpm test`
Expected: tsc/vite 构建通过,测试全绿

- [ ] **Step 4: 提交**

```bash
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore(release): bump version to 2.0.0 for the v2 UI

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 9: post-v1-issues.md 改造为 v2 待办文档

**Files:**
- Rename: `docs/post-v1-issues.md` → `docs/v2-issues.md`(用 `git mv`)
- Modify: 改造后的 `docs/v2-issues.md`

**Interfaces:**
- Consumes: 无
- Produces: v2 待办文档;仓库内无旧文件名引用(已验证),无需改引用

- [ ] **Step 1: git mv**

```bash
git mv docs/post-v1-issues.md docs/v2-issues.md
```

- [ ] **Step 2: 改标题与开头段**

标题 `# Post-v1 Issues` → `# CC Reminder v2 待办与遗留问题`。

首段替换为:

```markdown
CC Reminder v1 于 2026-08-26 合并入 main(d175fd0)。**v2 于 2026-08-26 以主界面重组开场**(7 页 → 4 页:工作台 / 通知规则 / 集成 / 设置,设计见 [docs/superpowers/specs/2026-08-26-main-ui-reorg-design.md](superpowers/specs/2026-08-26-main-ui-reorg-design.md),版本随重组升至 2.0.0)。以下条目在评审中被有意推迟(DEFER),按优先级分组记录;发布前无需全部完成,但标注「发布相关」的项应在首个 tag 前处理。
```

- [ ] **Step 3: 「项目架构整体优化与 UI 美化」条目补一句现状**

在该条目 **UI 美化** 子项末尾追加:

```markdown
  - v2 现状:导航层重组已完成(见上方 v2 开场);剩余为页内布局(HookRuleDrawer 930 行重组、历史筛选器精简)与视觉令牌体系,属本条目范围。
```

- [ ] **Step 4: 验证与提交**

Run: `ls docs/post-v1-issues.md 2>/dev/null; ls docs/v2-issues.md`
Expected: 仅后者存在

```bash
git add docs/v2-issues.md
git commit -m "docs: post-v1 issues becomes the v2 backlog

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 10: 全量验证收尾

**Files:** 无新改动(验证任务)

- [ ] **Step 1: 完整验证链**

Run: `pnpm verify`
Expected: vitest 全绿 + playwright 全绿(基线已固定)+ tsc/vite 构建通过

- [ ] **Step 2: 确认工作区干净**

Run: `git status --short && git log --oneline -10`
Expected: 工作区干净;约 9 个新提交(Task 1-9 各一),信息符合 conventional commits

---

## Self-Review 记录

- **Spec 覆盖**:§1 四页(Task 5/6)、§2 工作台+seed 下沉(Task 4/5)、§3 规则页(Task 5)、§4 集成页(Task 5)、§5 设置不动(Task 6 仅导航)、§6 AppShell/迁移/h1 降级(Task 3/6)、§7 TabBar(Task 1)、§8 i18n(Task 2/4)、§9 测试(Task 1-7 各步)、§10 版本(Task 8)、§11 v2 文档(Task 9)、§13 顺序=任务顺序。✓
- **占位符**:无 TBD/TODO;所有代码块完整。✓
- **类型一致性**:`PageId` 四值在 Task 6 定义、WorkbenchPage(Task 5)以 `import type` 前向引用(与现状 OverviewPage↔AppShell 同模式,类型级循环安全);`onOpenHistory?: (deliveryStatus?: DeliveryStatusCode) => void` 在 Task 4/5 两处一致;`gotoIntegrations/gotoRules/gotoHistoryTab` 键名在 Task 2 定义、Task 4 使用一致。✓
- **已知风险**:Task 3-6 之间存在唯一的跨任务红窗口(壳层 h1 断言),已在 Task 3 Step 2 说明并给出合并执行建议;Task 4 Step 5 说明了 AppShell 的最小联动改动避免 tsc 断裂。
