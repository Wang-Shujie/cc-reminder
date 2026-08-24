// Task 19 contract tests for the Overview page. The plan's Step 1 block is
// authoritative: the overview mirrors shared health (queue counts + issues),
// and issue action buttons navigate to the owning management page.
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { configuredBackend, type FakeBackend } from "../test/TestApp";
import { OverviewPage } from "./OverviewPage";

import type {
  ChannelId,
  HealthIssue,
  HealthSnapshot,
  HistoryItem,
} from "../lib/contracts";

function healthBackend(overrides: Partial<HealthSnapshot>): FakeBackend {
  return configuredBackend({ health: overrides });
}

function failedItem(): HistoryItem {
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
  };
}

test("overview presents the same health issues and queue counts as shared health", async () => {
  render(<OverviewPage backend={healthBackend({ failed_jobs: 2, pending_jobs: 4 })} />);
  expect(await screen.findByText("2 个失败任务")).toBeVisible();
  expect(screen.getByText("4 个待发送任务")).toBeVisible();
  expect(screen.getByRole("button", { name: "查看失败任务" })).toBeEnabled();
});

test("missing agent, drift and trust-pending issues navigate to Agent integrations", async () => {
  const onNavigate = vi.fn();
  const user = userEvent.setup();
  render(
    <OverviewPage
      backend={healthBackend({
        issues: [
          {
            issue_code: "agent.not_detected",
            level: "warning",
            message: "未检测到 Codex",
            suggested_command: null,
            suggested_action: null,
          },
          {
            issue_code: "hooks.selection_out_of_date",
            level: "warning",
            message: "Hook 配置与已安装的 Hook 不一致",
            suggested_command: null,
            suggested_action: null,
          },
          {
            issue_code: "integration.trust_pending",
            level: "warning",
            message: "Codex 需要确认 Hook",
            suggested_command: "/hooks",
            suggested_action: null,
          },
        ],
      })}
      onNavigate={onNavigate}
    />,
  );
  expect(await screen.findByText("未检测到 Codex")).toBeVisible();
  // The trust command is surfaced verbatim for the operator to copy.
  expect(screen.getByText("/hooks")).toBeVisible();
  const buttons = screen.getAllByRole("button", { name: "前往 Agent 集成" });
  expect(buttons).toHaveLength(3);
  await user.click(buttons[0]!);
  expect(onNavigate).toHaveBeenCalledWith("agents");
});

test("unavailable credential store and paused channel issues navigate to Channels", async () => {
  const onNavigate = vi.fn();
  const user = userEvent.setup();
  render(
    <OverviewPage
      backend={healthBackend({
        issues: [
          {
            issue_code: "credentials.store_unavailable",
            level: "error",
            message: "凭据存储不可用",
            suggested_command: null,
            suggested_action: "检查系统钥匙串设置",
          },
          {
            issue_code: "channel.auth_paused",
            level: "warning",
            message: "渠道「值班群」授权已暂停",
            suggested_command: null,
            suggested_action: null,
          },
        ],
      })}
      onNavigate={onNavigate}
    />,
  );
  expect(await screen.findByText("凭据存储不可用")).toBeVisible();
  expect(screen.getByText(/检查系统钥匙串设置/)).toBeVisible();
  const buttons = screen.getAllByRole("button", { name: "前往渠道" });
  expect(buttons).toHaveLength(2);
  await user.click(buttons[0]!);
  expect(onNavigate).toHaveBeenCalledWith("channels");
});

test("retry, expired, spool and rejected counts mirror the shared snapshot", async () => {
  render(
    <OverviewPage
      backend={healthBackend({
        retry_jobs: 1,
        expired_jobs: 5,
        spool_count: 3,
        rejected_count: 2,
      })}
    />,
  );
  expect(await screen.findByText("1 个等待重试任务")).toBeVisible();
  expect(screen.getByText("5 个过期任务")).toBeVisible();
  expect(screen.getByText("3 个暂存事件")).toBeVisible();
  expect(screen.getByText("2 个被拒绝事件")).toBeVisible();
});

test("last success time is shown; a never-succeeded queue says so", async () => {
  const { unmount } = render(
    <OverviewPage backend={healthBackend({ last_success_at: "2026-08-20T09:30:00+08:00" })} />,
  );
  expect(await screen.findByText(/上次成功/)).toBeVisible();
  expect(screen.getByText(/2026/)).toBeVisible();
  unmount();

  render(<OverviewPage backend={healthBackend({ last_success_at: null })} />);
  expect(await screen.findByText(/尚未成功/)).toBeVisible();
});

test("recent failures list comes from the failed delivery history", async () => {
  render(<OverviewPage backend={configuredBackend({ historyItems: [failedItem()] })} />);
  expect(await screen.findByText("PostToolUseFailure")).toBeVisible();
  expect(screen.getAllByText(/发送失败|failed/i).length).toBeGreaterThan(0);
});

test("a failed failures-query shows an error line, not 没有失败任务", async () => {
  const backend = configuredBackend({
    historyListError: { code: "storage.unavailable", message: "历史库暂不可用。" },
  });
  render(<OverviewPage backend={backend} />);
  expect(await screen.findByText("失败任务列表加载失败。")).toBeVisible();
  expect(screen.queryByText("最近没有失败任务。")).not.toBeInTheDocument();
});

test("查看失败任务 navigates to history pre-filtered to failed jobs", async () => {
  const onNavigate = vi.fn();
  const user = userEvent.setup();
  render(<OverviewPage backend={configuredBackend()} onNavigate={onNavigate} />);
  await user.click(await screen.findByRole("button", { name: "查看失败任务" }));
  expect(onNavigate).toHaveBeenCalledWith("history", { delivery_status: "failed" });
});

test("a failing health snapshot surfaces an alert instead of an empty board", async () => {
  const backend = configuredBackend({ snapshotError: true });
  render(<OverviewPage backend={backend} />);
  expect(await screen.findByRole("alert")).toHaveTextContent(/健康状态加载失败/);
});

test("core revision events refresh in the background through a polite live region", async () => {
  const backend = configuredBackend();
  render(<OverviewPage backend={backend} />);
  await screen.findByText("0 个失败任务");
  act(() => {
    backend.emit("core://queue-changed", { revision: 2 });
  });
  await waitFor(() => expect(backend.getHealthSnapshot).toHaveBeenCalledTimes(2));
  // Announced politely without moving focus.
  expect(screen.getByRole("status")).toHaveTextContent("数据已刷新");
  expect(document.activeElement).toBe(document.body);
});
