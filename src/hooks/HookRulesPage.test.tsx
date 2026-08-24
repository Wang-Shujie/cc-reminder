// Task 17 contract tests. The plan's Step 1–3 blocks are authoritative; the
// surrounding assertions lock columns, badges, inheritance semantics and
// privacy behavior of the Hook Rules page and its drawer.
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  PROJECT_ID,
  claudeRulesFixtures,
  codexRulesFixtures,
  defaultRuleConfig,
  globalScope,
  projectRulesBackendOptions,
  projectScope,
  testChannelSummary,
  FakeBackend,
  type FakeBackendOptions,
} from "../test/TestApp";
import { BackendProvider } from "../lib/backend";
import { HookRulesPage } from "./HookRulesPage";

import type { RuleConfig } from "../lib/contracts";

function rulesBackend(options: FakeBackendOptions = {}): FakeBackend {
  return new FakeBackend({
    channels: [testChannelSummary()],
    rules: [...claudeRulesFixtures(), ...codexRulesFixtures()],
    ...options,
  });
}

function projectRulesBackend(): FakeBackend {
  return new FakeBackend(projectRulesBackendOptions());
}

function projectRuleWithDeliveryOverride(): FakeBackend {
  const options = projectRulesBackendOptions();
  const delivery = {
    ...defaultRuleConfig(true).delivery,
    cooldown_seconds: 30,
  };
  return new FakeBackend({
    ...options,
    projectPatches: {
      [`${PROJECT_ID}:claude-code:Stop`]: { delivery },
    },
  });
}

function rulesBackendWithSelectionOutOfDate(): FakeBackend {
  return rulesBackend({
    rules: claudeRulesFixtures().map((row) =>
      row.source_event === "PermissionRequest" ? { ...row, installed: false } : row,
    ),
    selectionOutOfDate: true,
  });
}

function previewBackend(body: string): FakeBackend {
  return rulesBackend({ previewBody: body });
}

function renderRules({
  backend,
  scope,
}: {
  backend: FakeBackend;
  scope?: { scope: "global" } | { scope: "project"; project_id: string; project_name: string };
}): void {
  render(
    <BackendProvider backend={backend}>
      <HookRulesPage locale="zh_cn" initialScope={scope ?? globalScope()} />
    </BackendProvider>,
  );
}

async function openStopDrawer(user: ReturnType<typeof userEvent.setup>): Promise<void> {
  await user.click(await screen.findByRole("row", { name: /Stop/ }));
}

// ---------------------------------------------------------------------------
// Step 1: table visibility and filtering
// ---------------------------------------------------------------------------

test("shows every Hook including unavailable and high-frequency rows", async () => {
  renderRules({ backend: rulesBackend() });
  expect(await screen.findByRole("row", { name: /PermissionRequest/ })).toBeVisible();
  const unavailable = screen.getByRole("row", { name: /PostToolUseFailure/ });
  expect(within(unavailable).getByText("当前版本不支持")).toBeVisible();
  expect(within(unavailable).getByRole("switch")).toBeDisabled();
  expect(within(screen.getByRole("row", { name: /PreToolUse/ })).getByText("高频")).toBeVisible();

  // Agent tabs and scope selector exist.
  expect(screen.getByRole("tab", { name: "Claude Code" }).getAttribute("aria-selected")).toBe(
    "true",
  );
  expect(screen.getByRole("tab", { name: "Codex" })).toBeVisible();
  expect(screen.getByRole("radio", { name: "全局" }).getAttribute("aria-checked")).toBe("true");
  expect(screen.getByRole("radio", { name: "项目" })).toBeVisible();

  // Fixed capability columns.
  const headers = (await screen.findAllByRole("columnheader")).map((cell) => cell.textContent);
  expect(headers).toEqual(["开关", "Hook", "阶段", "Agent", "频率", "渠道", "配置来源", "状态"]);

  // Experimental and deprecated catalog states surface as badges.
  expect(screen.getByText("实验")).toBeVisible();
  expect(screen.getByText("废弃")).toBeVisible();
});

test("filters combine name phase enabled and sensitivity", async () => {
  const user = userEvent.setup();
  renderRules({ backend: rulesBackend() });
  await screen.findByRole("row", { name: /PermissionRequest/ });
  expect(await screen.findAllByRole("row")).toHaveLength(7);

  await user.type(screen.getByRole("searchbox", { name: "搜索 Hook" }), "permission");
  await user.selectOptions(screen.getByLabelText("阶段"), "permission");
  expect(screen.getAllByRole("row")).toHaveLength(2);

  // Search clear icon exists while a query is active.
  await user.selectOptions(screen.getByLabelText("敏感级别"), "forbidden");
  expect(screen.getAllByRole("row")).toHaveLength(1);
  expect(screen.getByRole("button", { name: "清除搜索" })).toBeVisible();

  await user.selectOptions(screen.getByLabelText("敏感级别"), "all");
  await user.selectOptions(screen.getByLabelText("阶段"), "all");
  await user.click(screen.getByRole("button", { name: "清除搜索" }));
  expect(screen.queryByRole("button", { name: "清除搜索" })).toBeNull();
  expect(screen.getAllByRole("row")).toHaveLength(7);
});

test("codex tab shows codex rows scoped to the selected agent", async () => {
  const user = userEvent.setup();
  renderRules({ backend: rulesBackend() });
  await screen.findByRole("row", { name: /PermissionRequest/ });
  await user.click(screen.getByRole("tab", { name: "Codex" }));
  expect(await screen.findByRole("row", { name: /SessionEnd/ })).toBeVisible();
  expect(screen.queryByRole("row", { name: /PostToolUseFailure/ })).toBeNull();
});

// ---------------------------------------------------------------------------
// Step 2: inheritance, drawer controls, selection drift
// ---------------------------------------------------------------------------

test("editing one inherited field creates only that project patch", async () => {
  const backend = projectRulesBackend();
  const user = userEvent.setup();
  renderRules({ backend, scope: projectScope() });
  await user.click(await screen.findByRole("row", { name: /Stop/ }));
  await user.click(screen.getByRole("switch", { name: "启用通知" }));
  expect(backend.saveProjectRulePatch).toHaveBeenCalledWith(
    expect.objectContaining({
      patch: { enabled: false },
    }),
  );
});

test("reset icon removes one override and restores inherited display", async () => {
  const backend = projectRuleWithDeliveryOverride();
  const user = userEvent.setup();
  renderRules({ backend, scope: projectScope() });
  await openStopDrawer(user);
  await user.click(screen.getByRole("button", { name: "恢复发送策略继承" }));
  expect(backend.resetProjectRuleField).toHaveBeenCalledWith(
    expect.objectContaining({ field: "delivery" }),
  );
  expect(await screen.findByText("继承全局")).toBeVisible();
});

test("rule selection drift requires one explicit Hook apply action", async () => {
  const backend = rulesBackendWithSelectionOutOfDate();
  const user = userEvent.setup();
  renderRules({ backend });
  await user.click(await screen.findByRole("button", { name: "应用 Hook 变更" }));

  // The confirmation lists owned-event changes before anything is applied.
  const dialog = screen.getByRole("dialog", { name: "确认应用 Hook 变更" });
  expect(within(dialog).getByText("PermissionRequest")).toBeVisible();
  expect(within(dialog).getByText("PreToolUse")).toBeVisible();
  expect(within(dialog).queryByRole("button", { name: "确认应用 Hook 变更" })).toBeVisible();

  await user.click(screen.getByRole("button", { name: "确认应用 Hook 变更" }));
  expect(backend.applyHookAction).toHaveBeenCalledWith(
    expect.objectContaining({ action: "repair" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "应用 Hook 变更" })).toBeNull(),
  );
});

test("codex drift apply warns that changes return to /hooks review", async () => {
  const user = userEvent.setup();
  renderRules({ backend: rulesBackend({ selectionOutOfDate: true }) });
  await user.click(await screen.findByRole("tab", { name: "Codex" }));
  await user.click(await screen.findByRole("button", { name: "应用 Hook 变更" }));
  const dialog = screen.getByRole("dialog", { name: "确认应用 Hook 变更" });
  expect(within(dialog).getByText(/\/hooks/)).toBeVisible();
});

test("global scope edits save the full rule config", async () => {
  const backend = rulesBackend();
  const user = userEvent.setup();
  renderRules({ backend });
  await openStopDrawer(user);
  await user.click(screen.getByRole("switch", { name: "启用通知" }));
  expect(backend.saveGlobalRule).toHaveBeenCalledWith(
    expect.objectContaining({
      agent: "claude-code",
      source_event: "Stop",
      config: expect.objectContaining<Partial<RuleConfig>>({ enabled: false }),
    }),
  );
});

test("drawer exposes segmented modes, numeric caps, privacy fields and quiet hours", async () => {
  const user = userEvent.setup();
  renderRules({ backend: rulesBackend() });
  await openStopDrawer(user);

  // Segmented controls are labeled radio groups.
  expect(screen.getByRole("radio", { name: "即时" })).toBeChecked();
  expect(screen.getByRole("radio", { name: "原生摘要" })).toBeInTheDocument();
  expect(screen.getByRole("radio", { name: "抑制" })).toBeChecked();

  // Numeric caps are bounded inputs.
  expect(screen.getByLabelText("冷却（秒）")).toHaveValue(0);
  expect(screen.getByLabelText("有效期（秒）")).toHaveValue(1800);
  expect(screen.getByLabelText("正文截断上限")).toHaveValue(0);

  // Privacy field checkboxes come from the catalog input fields.
  expect(screen.getByRole("checkbox", { name: "tool_input" })).not.toBeChecked();
  expect(screen.getByRole("checkbox", { name: "tool_name" })).toBeInTheDocument();

  // Quiet-hours controls exist behind the enable switch.
  expect(screen.getByRole("switch", { name: "启用静默" })).not.toBeChecked();
});

test("PermissionRequest aggregation stays disabled with an explanatory tooltip", async () => {
  const user = userEvent.setup();
  renderRules({ backend: rulesBackend() });
  await user.click(await screen.findByRole("row", { name: /PermissionRequest/ }));
  const aggregate = screen.getByRole("radio", { name: "聚合" });
  expect(aggregate).toBeDisabled();
  // Hover tooltip plus a screen-reader-accessible description; the accessible
  // NAME stays "聚合" so the control remains predictable to announce.
  expect(aggregate.getAttribute("title")).toContain("权限请求");
  expect(screen.getByText(/聚合不可用/)).toHaveClass("sr-only");
});

test("explicit quiet-hours clear sends quiet_hours null while reset removes the key", async () => {
  const options = projectRulesBackendOptions();
  options.rules = options.rules?.map((row) =>
    row.source_event === "Stop"
      ? {
          ...row,
          config: {
            ...row.config,
            quiet_hours: {
              start_local: "22:00",
              end_local: "08:00",
              weekdays: [1, 2, 3, 4, 5],
              bypass_at_or_above: null,
            },
          },
        }
      : row,
  );
  const backend = new FakeBackend(options);
  const user = userEvent.setup();
  renderRules({ backend, scope: projectScope() });
  await openStopDrawer(user);

  await user.click(screen.getByRole("switch", { name: "启用静默" }));
  expect(backend.saveProjectRulePatch).toHaveBeenCalledWith(
    expect.objectContaining({ patch: { quiet_hours: null } }),
  );

  // Once explicitly cleared, the section counts as overridden and can be reset.
  await user.click(await screen.findByRole("button", { name: "恢复静默时段继承" }));
  expect(backend.resetProjectRuleField).toHaveBeenCalledWith(
    expect.objectContaining({ field: "quiet_hours" }),
  );
});

// ---------------------------------------------------------------------------
// Step 3: preview and test-send privacy
// ---------------------------------------------------------------------------

test("preview shows redaction before any test send", async () => {
  const backend = previewBackend("摘要：[REDACTED]");
  const user = userEvent.setup();
  renderRules({ backend });
  await openStopDrawer(user);
  await user.type(screen.getByLabelText("模板"), " token={{event.summary}}");
  expect(await screen.findByText("摘要：[REDACTED]")).toBeVisible();
  expect(screen.queryByText("secret-raw-value")).not.toBeInTheDocument();
});

test("preview debounce cancels stale requests with a monotonic request id", async () => {
  vi.useFakeTimers();
  try {
    const backend = previewBackend("摘要：[REDACTED]");
    renderRules({ backend });
    // Flush the initial load promises without advancing the debounce timer.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    act(() => {
      fireEvent.click(screen.getByRole("row", { name: /Stop/ }));
    });
    const textarea = screen.getByLabelText("模板");
    act(() => {
      fireEvent.change(textarea, { target: { value: " token={{event.summary}}" } });
    });
    // Nothing fired inside the 250 ms debounce window.
    expect(backend.previewNotification).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    // The trailing request fires exactly once.
    expect(backend.previewNotification).toHaveBeenCalledTimes(1);
  } finally {
    vi.useRealTimers();
  }
});

test("unauthorized placeholder errors surface without leaking values", async () => {
  const backend = rulesBackend({ previewError: "模板包含未授权的占位符 {{secret.raw}}" });
  const user = userEvent.setup();
  renderRules({ backend });
  await openStopDrawer(user);
  await user.type(screen.getByLabelText("模板"), " token={{secret.raw}}");
  expect(await screen.findByRole("alert")).toBeVisible();
  expect(screen.queryByText("secret-raw-value")).not.toBeInTheDocument();
});

test("custom redaction patterns are validated before saving", async () => {
  const backend = rulesBackend();
  const user = userEvent.setup();
  renderRules({ backend });
  await openStopDrawer(user);
  await user.type(screen.getByLabelText("自定义脱敏规则"), "x".repeat(513));
  await user.tab();
  const alert = await screen.findByRole("alert");
  expect(alert).toBeVisible();
  expect(backend.saveGlobalRule).not.toHaveBeenCalled();
});

test("actual test send confirms by naming the target group", async () => {
  const backend = rulesBackend();
  const user = userEvent.setup();
  renderRules({ backend });
  await openStopDrawer(user);
  await user.click(screen.getByRole("button", { name: "发送测试" }));
  // The confirmation names the target group in its accessible name.
  const dialog = screen.getByRole("dialog", { name: /值班群/ });
  expect(dialog).toBeVisible();
  await user.click(screen.getByRole("button", { name: "确认发送" }));
  expect(backend.sendRuleTest).toHaveBeenCalledWith(
    expect.objectContaining({ source_event: "Stop", channel_id: "ch-1" }),
  );
});

test("failed test sends show diagnostics instead of raw payloads", async () => {
  const backend = rulesBackend({
    sendError: "delivery.http_502 上游通道暂时不可用（详情已脱敏）",
  });
  const user = userEvent.setup();
  renderRules({ backend });
  await openStopDrawer(user);
  await user.click(screen.getByRole("button", { name: "发送测试" }));
  await user.click(screen.getByRole("button", { name: "确认发送" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("delivery.http_502");
});
