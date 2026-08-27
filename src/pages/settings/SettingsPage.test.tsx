// Settings page contract tests. The plan's Step 3 block is authoritative:
// native controls persisting exact values. Task 20 fix round 1 changed the
// export path contract — exportDiagnostics() takes NO argument (the save
// dialog opens inside the core) — and added the bounded debug-logging select.
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { configuredBackend, type FakeBackend } from "../../test/TestApp";
import { SettingsPage } from "./SettingsPage";

import type { UpdateCheckResult } from "../../lib/contracts";

function settingsBackend(extra = {}): FakeBackend {
  return configuredBackend(extra);
}

function deferred(): {
  promise: Promise<UpdateCheckResult>;
  resolve: (value: UpdateCheckResult) => void;
} {
  let resolve!: (value: UpdateCheckResult) => void;
  const promise = new Promise<UpdateCheckResult>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

test("settings use native controls and persist exact values", async () => {
  const backend = settingsBackend();
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  await user.click(screen.getByRole("checkbox", { name: "开机启动" }));
  await user.selectOptions(screen.getByLabelText("语言"), "en");
  await user.clear(screen.getByLabelText("历史保留天数"));
  await user.type(screen.getByLabelText("历史保留天数"), "14");
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(backend.saveSettings).toHaveBeenCalledWith(
    expect.objectContaining({ autostart: true, locale: "en", event_retention_days: 14 }),
  );
});

test("close-to-tray and the theme radio group persist too", async () => {
  const backend = settingsBackend();
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  await user.click(screen.getByRole("checkbox", { name: "关闭时最小化到托盘" }));
  await user.click(screen.getByRole("radio", { name: "深色" }));
  await user.clear(screen.getByLabelText("历史保留天数"));
  await user.type(screen.getByLabelText("历史保留天数"), "30");
  await user.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(backend.saveSettings).toHaveBeenCalledWith(
      expect.objectContaining({ close_to_tray: false, theme: "dark" }),
    ),
  );
});

test("retention days outside 1-365 are rejected before saving", async () => {
  const backend = settingsBackend();
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  const days = await screen.findByLabelText("历史保留天数");
  await user.clear(days);
  await user.type(days, "366");
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(backend.saveSettings).not.toHaveBeenCalled();
  expect(screen.getByRole("alert")).toHaveTextContent(/1–365/);
});

test("pause uses setNotificationPause; resume clears it and shows paused_until", async () => {
  const backend = settingsBackend();
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "暂停 15 分钟" }));
  await waitFor(() =>
    expect(backend.setNotificationPause).toHaveBeenCalledWith({
      duration: "fifteen_minutes",
      // Browser UTC offset in east-positive seconds so the core can compute
      // the local midnight for 暂停至今日.
      offset_seconds: -new Date().getTimezoneOffset() * 60,
    }),
  );
  expect(await screen.findByText(/暂停至：/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "恢复通知" }));
  await waitFor(() => expect(backend.clearNotificationPause).toHaveBeenCalledTimes(1));
  expect(await screen.findByText("通知未暂停。")).toBeVisible();
});

test("a changed language discloses that it applies after a restart", async () => {
  const backend = settingsBackend();
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  // Hydrated saved locale matches the applied one: no hint.
  await screen.findByLabelText("语言");
  expect(screen.queryByText(/重启后生效/)).not.toBeInTheDocument();
  await user.selectOptions(screen.getByLabelText("语言"), "en");
  expect(screen.getByText("语言将在重启应用后生效。")).toBeVisible();
});

test("update check reports up-to-date when nothing is available", async () => {
  const backend = settingsBackend();
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "检查更新" }));
  expect(await screen.findByText("已是最新版本。")).toBeVisible();
});

test("an available update installs only after explicit confirmation", async () => {
  const backend = settingsBackend({
    updateCheck: {
      available: true,
      version: "1.2.0",
      notes: "修复投递稳定性。",
      installable: true,
    },
  });
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "检查更新" }));
  expect(await screen.findByText(/发现新版本：/)).toBeVisible();
  expect(screen.getByText("1.2.0")).toBeVisible();
  expect(screen.getByText("修复投递稳定性。")).toBeVisible();

  await user.click(screen.getByRole("button", { name: "安装更新" }));
  const dialog = screen.getByRole("dialog", { name: "确认安装更新" });
  expect(dialog).toBeVisible();
  // Focus moves to the confirm button when the dialog opens.
  expect(screen.getByRole("button", { name: "确认安装" })).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "确认安装" }));
  await waitFor(() => expect(backend.installUpdate).toHaveBeenCalledWith({ confirmed: true }));
  expect(await screen.findByText("更新程序已启动。")).toBeVisible();
  // Focus returned to the initiating control after the dialog closed.
  expect(screen.getByRole("button", { name: "检查更新" })).toHaveFocus();
});

test("a running check disables its button (no duplicate submission)", async () => {
  const backend = settingsBackend();
  const pending = deferred();
  const checkSpy = vi.fn(() => pending.promise);
  Object.defineProperty(backend, "checkForUpdates", { value: checkSpy });
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "检查更新" }));
  expect(screen.getByRole("button", { name: "检查中…" })).toBeDisabled();
  pending.resolve({ available: false, version: null, notes: null, installable: false });
  expect(await screen.findByText("已是最新版本。")).toBeVisible();
  expect(screen.getByRole("button", { name: "检查更新" })).toBeEnabled();
});

test("an unavailable secure credential store is disclosed on this page", async () => {
  const backend = settingsBackend({
    health: {
      issues: [
        {
          issue_code: "credentials.store_unavailable",
          level: "warning",
          message: "凭据存储不可用，已改用内存存储。",
          suggested_command: null,
          suggested_action: "检查系统钥匙串访问权限。",
        },
      ],
    },
  });
  render(<SettingsPage backend={backend} />);
  expect(await screen.findByText(/凭据存储不可用，已改用内存存储。/)).toBeVisible();
  expect(screen.getByText(/检查系统钥匙串访问权限。/)).toBeVisible();
});

test("exports diagnostics and clears only inactive history after confirmation", async () => {
  const user = userEvent.setup();
  const backend = settingsBackend();
  render(<SettingsPage backend={backend} />);
  // 无页题(立柱定位):以诊断按钮出现为就绪信号。
  await screen.findByRole("button", { name: "导出诊断" });

  // The save dialog lives in the core: the page sends NO argument at all.
  await user.click(screen.getByRole("button", { name: "导出诊断" }));
  await waitFor(() => expect(backend.exportDiagnostics).toHaveBeenCalledWith());
  expect(await screen.findByText(/诊断包已保存：/)).toBeVisible();

  await user.click(screen.getByRole("button", { name: "清除历史" }));
  const dialog = screen.getByRole("dialog");
  expect(dialog).toHaveTextContent(/活动任务及其事件将被保留/);
  await user.click(screen.getByRole("button", { name: "确认清除历史" }));
  await waitFor(() =>
    expect(backend.clearHistory).toHaveBeenCalledWith({ preserve_active_jobs: true }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

test("a cancelled export discloses the cancellation instead of a path", async () => {
  const backend = settingsBackend();
  Object.defineProperty(backend, "exportDiagnostics", {
    value: vi.fn(async () => ({ status: "cancelled" as const })),
  });
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "导出诊断" }));
  expect(await screen.findByText("已取消导出。")).toBeVisible();
});

test("debug logging select maps 关闭/15 分钟/60 分钟 to setDebugLogging durations", async () => {
  const backend = settingsBackend();
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);

  const select = await screen.findByLabelText("调试日志时长");
  await user.selectOptions(select, "15");
  await waitFor(() =>
    expect(backend.setDebugLogging).toHaveBeenCalledWith({ duration_minutes: 15 }),
  );

  await user.selectOptions(select, "60");
  await waitFor(() =>
    expect(backend.setDebugLogging).toHaveBeenCalledWith({ duration_minutes: 60 }),
  );

  await user.selectOptions(select, "0");
  await waitFor(() =>
    expect(backend.setDebugLogging).toHaveBeenCalledWith({ duration_minutes: 0 }),
  );
});
