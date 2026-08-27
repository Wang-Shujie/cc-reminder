// Task 19 contract tests for the Overview page. The plan's Step 1 block is
// authoritative: the overview mirrors shared health (queue counts + issues),
// and issue action buttons navigate to the owning management page.
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { configuredBackend, type FakeBackend } from "../../test/TestApp";
import { OverviewPage } from "./OverviewPage";

import type {
  ChannelId,
  HealthIssue,
  HealthSnapshot,
  HistoryItem,
} from "../../lib/contracts";

/** The metric plate carrying this label (wayfinding moment-board markup). */
function plate(label: string): HTMLElement {
  const el = screen.getByText(label).closest("li");
  if (el === null) {
    throw new Error(`metric plate labeled ${label} not found`);
  }
  return el;
}

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
  expect(await screen.findByText("失败任务")).toBeVisible();
  expect(within(plate("失败任务")).getByText("2")).toBeVisible();
  expect(within(plate("待发送")).getByText("4")).toBeVisible();
});

test("missing agent, drift and trust-pending issues navigate to Integrations", async () => {
  const onNavigate = vi.fn();
  const onOpenHistory = vi.fn();
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
      onOpenHistory={onOpenHistory}
    />,
  );
  expect(await screen.findByText("未检测到 Codex")).toBeVisible();
  // The trust command is surfaced verbatim for the operator to copy.
  expect(screen.getByText("/hooks")).toBeVisible();
  const integrationButtons = screen.getAllByRole("button", { name: "前往集成" });
  expect(integrationButtons).toHaveLength(2);
  // hooks.* 前缀的 issue 归通知规则页。
  const rulesButton = screen.getByRole("button", { name: "前往规则" });
  await user.click(integrationButtons[0]!);
  expect(onNavigate).toHaveBeenCalledWith("integrations");
  await user.click(rulesButton);
  expect(onNavigate).toHaveBeenCalledWith("rules");
});

test("unavailable credential store and paused channel issues navigate to Integrations", async () => {
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
  const buttons = screen.getAllByRole("button", { name: "前往集成" });
  expect(buttons).toHaveLength(2);
  await user.click(buttons[0]!);
  expect(onNavigate).toHaveBeenCalledWith("integrations");
});

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

test("retry and expired counts mirror the shared snapshot", async () => {
  render(
    <OverviewPage
      backend={healthBackend({
        retry_jobs: 1,
        expired_jobs: 5,
      })}
    />,
  );
  expect(await screen.findByText("等待重试")).toBeVisible();
  expect(within(plate("等待重试")).getByText("1")).toBeVisible();
  expect(within(plate("已过期")).getByText("5")).toBeVisible();
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

test("a failing health snapshot surfaces an alert instead of an empty board", async () => {
  const backend = configuredBackend({ snapshotError: true });
  render(<OverviewPage backend={backend} />);
  expect(await screen.findByRole("alert")).toHaveTextContent(/健康状态加载失败/);
});

test("core revision events refresh in the background through a polite live region", async () => {
  const backend = configuredBackend();
  render(<OverviewPage backend={backend} />);
  await screen.findByText("失败任务");
  act(() => {
    backend.emit("core://queue-changed", { revision: 2 });
  });
  await waitFor(() => expect(backend.getHealthSnapshot).toHaveBeenCalledTimes(2));
  // Announced politely without moving focus.
  expect(screen.getByRole("status")).toHaveTextContent("数据已刷新");
  expect(document.activeElement).toBe(document.body);
});
