// Task 18 contract tests for the Agent Integration page. The plan's Step 1
// block is authoritative; surrounding assertions lock action→backend mapping,
// the compatible-version consent flow, Codex /hooks trust handling, loading
// states and redacted actionable errors.
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

import {
  claudeRulesFixtures,
  codexRulesFixtures,
  configuredBackend,
  type FakeBackend,
  type FakeBackendOptions,
} from "../test/TestApp";
import { AgentsPage } from "./AgentsPage";

import type { AgentIntegrationSummary, HookInstallationResult } from "../lib/contracts";

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
  // NeedsUserConfirmation → the official command with copy + recheck.
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
