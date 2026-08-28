// Task 18 contract tests for the Agent Integration page. The plan's Step 1
// block is authoritative; surrounding assertions lock action→backend mapping,
// the compatible-version consent flow, Codex /hooks trust handling, loading
// states and redacted actionable errors.
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

import {
  claudeRulesFixtures,
  codexRulesFixtures,
  configuredBackend,
  type FakeBackend,
  type FakeBackendOptions,
} from "../../test/TestApp";
import { AgentsPage } from "./AgentsPage";

import type { AgentIntegrationSummary, HookInstallationResult } from "../../lib/contracts";

function agentsBackend(options: FakeBackendOptions = {}): FakeBackend {
  return configuredBackend({
    rules: [...claudeRulesFixtures(), ...codexRulesFixtures()],
    ...options,
  });
}

const CLAUDE_DETECTED: AgentIntegrationSummary = {
  agent: "claude-code",
  installed: true,
  version: "2.1.218",
  executable_path: "/usr/local/bin/claude",
  health: "detected",
  needs_compatible_version_confirmation: false,
};

const CODEX_DETECTED: AgentIntegrationSummary = {
  agent: "codex",
  installed: true,
  version: "0.145.0",
  executable_path: "/usr/local/bin/codex",
  health: "detected",
  needs_compatible_version_confirmation: false,
};

/** Plan Step 1 fixture: Claude Code owns drift while Codex sits on an
 *  unverified (unknown-major) version. */
function agentsBackendWithDriftAndUnknownMajor(): FakeBackend {
  return agentsBackend({
    detectResults: () => [
      CLAUDE_DETECTED,
      { ...CODEX_DETECTED, version: "9.9.9", needs_compatible_version_confirmation: true },
    ],
    rules: [
      ...claudeRulesFixtures().map((row) =>
        row.source_event === "Stop" ? { ...row, installed: false } : row,
      ),
      ...codexRulesFixtures(),
    ],
  });
}

/** Both agents detected with a consistent rule selection (steady state). */
function installedAgentsBackend(): FakeBackend {
  return agentsBackend();
}

test("drift offers repair while unknown major offers application upgrade only", async () => {
  render(<AgentsPage backend={agentsBackendWithDriftAndUnknownMajor()} />);
  expect(await screen.findByRole("button", { name: "修复 Claude Code Hook" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "安装 Codex Hook" })).toBeDisabled();
  expect(screen.getByText("需要升级 CC Reminder")).toBeVisible();
});

test("uninstall confirmation states that foreign Hooks remain", async () => {
  const user = userEvent.setup();
  const backend = installedAgentsBackend();
  render(<AgentsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "卸载 Claude Code Hook" }));
  expect(screen.getByRole("dialog")).toHaveTextContent("只移除 CC Reminder 创建的 Hook");

  // Confirming maps to the exact closed action enum on the backend call.
  await user.click(screen.getByRole("button", { name: "确认卸载" }));
  await waitFor(() =>
    expect(backend.applyHookAction).toHaveBeenCalledWith(
      expect.objectContaining({ agent: "claude-code", action: "uninstall" }),
    ),
  );
});

test("install maps to apply_hook_action with the closed install action", async () => {
  const user = userEvent.setup();
  const backend = agentsBackend({
    // Claude Code installed but no hooks registered yet → install offered.
    detectResults: () => [CLAUDE_DETECTED, CODEX_DETECTED],
    rules: [...codexRulesFixtures()],
  });
  render(<AgentsPage backend={backend} />);
  // Settle the initial detection fetch so the row reflects installed state.
  await screen.findByText("2.1.218");
  const install = screen.getByRole("button", { name: "安装 Claude Code Hook" });
  expect(install).toBeEnabled();
  await user.click(install);
  await waitFor(() =>
    expect(backend.applyHookAction).toHaveBeenCalledWith(
      expect.objectContaining({
        agent: "claude-code",
        action: "install",
        confirm_compatible_version: false,
      }),
    ),
  );
});

test("compatible-version rejection discloses, then retries with confirmation", async () => {
  const user = userEvent.setup();
  const gated = agentsBackend({ applyConfirmationRequired: true });
  render(<AgentsPage backend={gated} />);
  const repair = await screen.findByRole("button", { name: "修复 Claude Code Hook" });
  await user.click(repair);
  // First attempt goes out WITHOUT consent…
  expect(gated.applyHookAction).toHaveBeenCalledWith(
    expect.objectContaining({ confirm_compatible_version: false }),
  );
  // …then the disclosure appears and confirming retries with true.
  expect(await screen.findByText(/尚未经精确验证/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "确认继续" }));
  await waitFor(() =>
    expect(gated.applyHookAction).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({ confirm_compatible_version: true }),
    ),
  );
});

test("applied result shows per-event health and Codex /hooks copy/recheck on trust pending", async () => {
  const user = userEvent.setup();
  const backend = agentsBackend({
    detectResults: () => [
      CLAUDE_DETECTED,
      { ...CODEX_DETECTED, installed: false },
    ],
    rules: [],
    applyEntries: () => [
      { source_event: "Stop", trust_status: "needs_user_confirmation", health: "healthy" },
    ],
  });
  render(<AgentsPage backend={backend} />);
  // Settle the initial detection fetch before targeting the button.
  await screen.findByText("0.145.0");
  await user.click(screen.getByRole("button", { name: "安装 Codex Hook" }));
  // Per-event health display of the applied result.
  expect(await screen.findByText(/Stop · 健康/)).toBeVisible();
  // NeedsUserConfirmation → the guidance moved into a dialog (user decision):
  // the inline row only carries the entry link.
  await user.click(screen.getByRole("button", { name: "查看指引" }));
  expect(await screen.findByRole("dialog", { name: "Codex 信任指引" })).toBeVisible();
  expect(screen.getByText("/hooks")).toBeVisible();
  const detectionsBefore = vi.mocked(backend.detectAgents).mock.calls.length;
  await user.click(screen.getByRole("button", { name: "复制命令" }));
  await user.click(screen.getByRole("button", { name: "重新检测" }));
  await waitFor(() =>
    expect(vi.mocked(backend.detectAgents).mock.calls.length).toBeGreaterThan(
      detectionsBefore,
    ),
  );
});

test("once one Codex entry is observed, pending entries read as awaiting-first-run", async () => {
  const user = userEvent.setup();
  const backend = agentsBackend({
    detectResults: () => [
      CLAUDE_DETECTED,
      { ...CODEX_DETECTED, installed: false },
    ],
    rules: [],
    applyEntries: () => [
      { source_event: "Stop", trust_status: "observed_working", health: "healthy" },
      {
        source_event: "PermissionRequest",
        trust_status: "needs_user_confirmation",
        health: "healthy",
      },
    ],
  });
  render(<AgentsPage backend={backend} />);
  await screen.findByText("0.145.0");
  await user.click(screen.getByRole("button", { name: "安装 Codex Hook" }));
  // The pending entry is labelled "waiting for its first real occurrence"…
  expect(
    await screen.findByText(/PermissionRequest · 健康 · 等待首次触发/),
  ).toBeVisible();
  // …with a copyable suggested prompt (v2-issues:用户裁决 2026-08-28)…
  expect(screen.getByText(/建议提示词：/)).toBeVisible();
  expect(
    screen.getByRole("button", { name: "复制命令 PermissionRequest" }),
  ).toBeVisible();
  // …and the actionable /hooks instruction is replaced by the informational
  // notice — no command, no copy/recheck buttons.
  expect(screen.queryByText("/hooks")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "复制命令" })).not.toBeInTheDocument();
  expect(screen.getByText(/官方确认已完成/)).toBeVisible();
});

test("the suggested prompt copies to the clipboard on click", async () => {
  // userEvent.setup() 自带 clipboard stub(覆盖 jsdom 的 getter-only 属性),
  // 写入后经 readText 读回验证完整链路。
  const user = userEvent.setup();
  const backend = agentsBackend({
    detectResults: () => [CLAUDE_DETECTED, { ...CODEX_DETECTED, installed: false }],
    rules: [],
    applyEntries: () => [
      { source_event: "Stop", trust_status: "observed_working", health: "healthy" },
      {
        source_event: "PermissionRequest",
        trust_status: "needs_user_confirmation",
        health: "healthy",
      },
    ],
  });
  render(<AgentsPage backend={backend} />);
  await screen.findByText("0.145.0");
  await user.click(screen.getByRole("button", { name: "安装 Codex Hook" }));
  await user.click(
    await screen.findByRole("button", { name: "复制命令 PermissionRequest" }),
  );
  expect(await navigator.clipboard.readText()).toBe(
    "请运行 ls -la / 命令（触发权限确认，选择允许）",
  );
});

test("actions disable while running and recover after completion", async () => {
  const user = userEvent.setup();
  const backend = installedAgentsBackend();
  let release!: (value: HookInstallationResult) => void;
  vi.mocked(backend.applyHookAction).mockImplementationOnce(
    () =>
      new Promise<HookInstallationResult>((resolve) => {
        release = resolve;
      }),
  );
  render(<AgentsPage backend={backend} />);
  const repair = await screen.findByRole("button", { name: "修复 Claude Code Hook" });
  await user.click(repair);
  expect(screen.getByRole("button", { name: "修复 Claude Code Hook" })).toBeDisabled();
  release({ agent: "claude-code", selection_out_of_date: false, entries: [] });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "修复 Claude Code Hook" })).toBeEnabled(),
  );
});

test("errors surface the redacted message plus suggested action", async () => {
  const user = userEvent.setup();
  const backend = agentsBackend({
    applyError: {
      code: "integration.helper_mismatch",
      message: "Helper 版本不匹配，已拒绝修改 Hook。",
      suggested_action: "先升级 CC Reminder，再执行修复。",
    },
  });
  render(<AgentsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "修复 Claude Code Hook" }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("Helper 版本不匹配，已拒绝修改 Hook。");
  expect(alert).toHaveTextContent("先升级 CC Reminder，再执行修复。");
});

test("repair label appears only when the action really is repair; drift-only shows install", async () => {
  // Claude Code installed but NO hooks installed while events are enabled →
  // the action is "install", so the button must not claim to repair.
  const user = userEvent.setup();
  const backend = agentsBackend({
    rules: claudeRulesFixtures().map((row) => ({
      ...row,
      agent: "claude-code" as const,
      enabled: true,
      installed: false,
    })),
  });
  render(<AgentsPage backend={backend} />);
  // Settle the initial load so the button node is the final one.
  await screen.findByText("2.1.218");
  const button = screen.getByRole("button", { name: "安装 Claude Code Hook" });
  expect(button).toBeEnabled();
  expect(
    screen.queryByRole("button", { name: /修复 Claude Code Hook/ }),
  ).toBeNull();
  await user.click(button);
  await waitFor(() =>
    expect(backend.applyHookAction).toHaveBeenCalledWith(
      expect.objectContaining({ agent: "claude-code", action: "install" }),
    ),
  );
});

test("version-consent confirm disables while applying", async () => {
  const user = userEvent.setup();
  const gated = agentsBackend({ applyConfirmationRequired: true });
  render(<AgentsPage backend={gated} />);
  await user.click(await screen.findByRole("button", { name: "修复 Claude Code Hook" }));
  await screen.findByText(/尚未经精确验证/);
  let release!: (value: HookInstallationResult) => void;
  vi.mocked(gated.applyHookAction).mockImplementationOnce(
    () =>
      new Promise<HookInstallationResult>((resolve) => {
        release = resolve;
      }),
  );
  await user.click(screen.getByRole("button", { name: "确认继续" }));
  expect(screen.getByRole("button", { name: "确认继续" })).toBeDisabled();
  release({ agent: "claude-code", selection_out_of_date: false, entries: [] });
  // The dialog closes once the retried apply succeeds.
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "确认继续" })).toBeNull(),
  );
});

test("failed primary integration list surfaces an incomplete-data alert", async () => {
  const backend = agentsBackend();
  backend.listAgentIntegrations = async () => {
    throw { code: "internal_error", message: "boom" };
  };
  render(<AgentsPage backend={backend} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("列表加载失败");
});

test("successful uninstall clears stale applied-entry health", async () => {
  const user = userEvent.setup();
  const backend = agentsBackend({
    applyEntries: () => [
      { source_event: "Stop", trust_status: "not_required", health: "healthy" },
    ],
  });
  render(<AgentsPage backend={backend} />);
  await screen.findByText("2.1.218");
  // 升级 Helper exists per agent row; scope to the Claude Code row.
  const claudeRow = screen.getByText("Claude Code").closest("tr")!;
  await user.click(within(claudeRow).getByRole("button", { name: "升级 Helper" }));
  expect(await screen.findByText(/Stop · 健康/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "卸载 Claude Code Hook" }));
  await user.click(screen.getByRole("button", { name: "确认卸载" }));
  await waitFor(() =>
    expect(screen.queryByText(/最近应用结果/)).toBeNull(),
  );
  expect(screen.queryByText(/Stop · 健康/)).toBeNull();
});
