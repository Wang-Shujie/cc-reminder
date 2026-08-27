// Task 19 contract tests for the Notification History page. The plan's Step 2
// blocks are authoritative: result filtering, redacted-only detail, and manual
// retry eligibility (failed retryable, expired NOT).
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  PROJECT_ID,
  configuredBackend,
  testChannelSummary,
  type FakeBackend,
} from "../../test/TestApp";
import { HistoryPage } from "./HistoryPage";

import type {
  ChannelId,
  DeliveryAttemptDto,
  GetHistoryDetailInput,
  HistoryItem,
  HistoryPage as HistoryPageDto,
  ListHistoryInput,
  ProjectId,
  ProjectSummary,
} from "../../lib/contracts";
import type { FakeBackendOptions } from "../../test/TestApp";

function demoProject(): ProjectSummary {
  return {
    id: PROJECT_ID as ProjectId,
    name: "演示项目",
    canonical_root: "/work/main",
    worktree_mode: "alias",
    paths: [],
    override_count: 0,
  };
}

function mkItem(
  event_id: string,
  overrides: Partial<HistoryItem> = {},
): HistoryItem {
  return {
    event_id,
    source: "claude-code",
    source_version: "2.1.218",
    source_event: "PreToolUse",
    category: "tool",
    occurred_at: "2026-08-20T10:00:00+08:00",
    received_at: "2026-08-20T10:00:01+08:00",
    project_id: PROJECT_ID as ProjectId,
    project_display_name: "演示项目",
    unmatched_cwd_fingerprint: null,
    model: "claude-opus-4",
    permission_mode: "default",
    severity: "info",
    public_fields: { tool_name: "Bash" },
    correlation_id: `corr-${event_id}`,
    processing_outcome: "queued",
    outcome_reason_code: null,
    delivery_job_id: `job-${event_id}`,
    channel_id: "ch-1" as ChannelId,
    document: {
      title: "工具调用",
      severity: "info",
      facts: [["工具", "Bash"]],
      body: "已执行工具调用。",
      footer: null,
    },
    delivery_status: "pending",
    attempts: [],
    ...overrides,
  };
}

function stopFailure(): HistoryItem {
  const attempt: DeliveryAttemptDto = {
    attempt_number: 1,
    started_at: "2026-08-20T10:00:05+08:00",
    completed_at: "2026-08-20T10:00:06+08:00",
    outcome: "rejected_by_platform",
    http_status: 500,
    platform_code: null,
    error_code: "network.timeout",
    retry_at: "2026-08-20T10:05:00+08:00",
    redacted_detail: "请求失败：[REDACTED]",
  };
  return mkItem("evt-stop", {
    source_event: "Stop",
    category: "lifecycle",
    severity: "info",
    project_id: null,
    project_display_name: null,
    unmatched_cwd_fingerprint: "sha256:9f2cabc123",
    delivery_status: "failed",
    document: {
      title: "会话结束",
      severity: "info" as const,
      facts: [["工具输入", "[REDACTED]"] as [string, string]],
      body: "会话已结束。工具输入：[REDACTED]",
      footer: "CC Reminder",
    },
    attempts: [attempt],
  });
}

function historyBackend(items: HistoryItem[], extra = {}): FakeBackend {
  return configuredBackend({
    historyItems: items,
    channels: [testChannelSummary()],
    projects: [demoProject()],
    ...extra,
  });
}

/** One failed, one expired, one succeeded job: retry eligibility contrast. */
function historyWithFailedExpiredAndSucceeded(
  extra: FakeBackendOptions = {},
): FakeBackend {
  return historyBackend(
    [
      mkItem("evt-ok", { source_event: "PreToolUse", delivery_status: "succeeded" }),
      mkItem("evt-expired", {
        source_event: "SessionEnd",
        delivery_status: "expired",
        processing_outcome: "expired",
        outcome_reason_code: "ttl_exceeded",
      }),
      mkItem("evt-failed", {
        source_event: "Stop",
        category: "lifecycle",
        delivery_status: "failed",
        document: {
          title: "会话结束",
          severity: "info" as const,
          facts: [],
          body: "[REDACTED]",
          footer: null,
        },
      }),
    ],
    extra,
  );
}

test("history filters and detail show only redacted content", async () => {
  const backend = historyBackend([stopFailure(), mkItem("evt-a"), mkItem("evt-b")]);
  const user = userEvent.setup();
  render(<HistoryPage backend={backend} />);
  await user.selectOptions(screen.getByLabelText("结果"), "failed");
  await user.click(await screen.findByRole("row", { name: /Stop.*失败/ }));
  expect(screen.getByRole("dialog")).toHaveTextContent("[REDACTED]");
  expect(document.body.textContent).not.toContain("secret-raw-value");
});

test("manual retry is available only for eligible failed jobs", async () => {
  // NOTE: one added await versus the plan block — promise-backed rows cannot
  // exist synchronously after render (verified empirically); the contract
  // assertions themselves are unchanged.
  render(<HistoryPage backend={historyWithFailedExpiredAndSucceeded()} />);
  expect(await screen.findByRole("button", { name: "重试失败任务" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "重试过期任务" })).toBeDisabled();
});

test("time/project/Hook/channel filters reach the backend", async () => {
  const backend = historyBackend([stopFailure()]);
  const user = userEvent.setup();
  render(<HistoryPage backend={backend} />);
  await screen.findByRole("table");

  // Bare <input type="date"> values are transformed to whole-day RFC3339
  // bounds so the core's DateTime<Utc> can parse them.
  fireEvent.change(screen.getByLabelText("时间从"), { target: { value: "2026-08-01" } });
  await waitFor(() =>
    expect(backend.listHistory).toHaveBeenCalledWith(
      expect.objectContaining({ occurred_from: "2026-08-01T00:00:00Z" }),
    ),
  );
  fireEvent.change(screen.getByLabelText("时间至"), { target: { value: "2026-08-31" } });
  await waitFor(() =>
    expect(backend.listHistory).toHaveBeenCalledWith(
      expect.objectContaining({ occurred_until: "2026-08-31T23:59:59Z" }),
    ),
  );
  await user.selectOptions(screen.getByLabelText("项目"), PROJECT_ID);
  await waitFor(() =>
    expect(backend.listHistory).toHaveBeenCalledWith(
      expect.objectContaining({ project_id: PROJECT_ID }),
    ),
  );
  await user.selectOptions(screen.getByLabelText("渠道"), "ch-1");
  await waitFor(() =>
    expect(backend.listHistory).toHaveBeenCalledWith(
      expect.objectContaining({ channel_id: "ch-1" }),
    ),
  );
  // The Hook filter is free-text committed with Enter.
  await user.type(screen.getByLabelText("Hook"), "Stop{Enter}");
  await waitFor(() =>
    expect(backend.listHistory).toHaveBeenCalledWith(
      expect.objectContaining({ source_event: "Stop" }),
    ),
  );
});

test("pagination honors next_offset and disappears at the end", async () => {
  const items = Array.from({ length: 12 }, (_, i) => mkItem(`evt-${i}`));
  const backend = historyBackend(items);
  const user = userEvent.setup();
  render(<HistoryPage backend={backend} />);

  // Wait for actual data rows (the bare header row exists immediately).
  await screen.findAllByRole("row", { name: /PreToolUse/ });
  const rowsAfterFirstPage = screen.getAllByRole("row");
  expect(rowsAfterFirstPage.length - 1).toBe(10); // header excluded
  expect(backend.listHistory).toHaveBeenCalledWith(
    expect.objectContaining({ offset: 0, limit: 10 }),
  );

  await user.click(screen.getByRole("button", { name: "加载更多" }));
  await waitFor(() => expect(screen.getAllByRole("row").length - 1).toBe(12));
  expect(backend.listHistory).toHaveBeenCalledWith(
    expect.objectContaining({ offset: 10, limit: 10 }),
  );
  expect(screen.queryByRole("button", { name: "加载更多" })).not.toBeInTheDocument();
});

test("detail shows attempt timeline and only the fingerprint for unmatched cwd", async () => {
  const user = userEvent.setup();
  render(<HistoryPage backend={historyBackend([stopFailure()])} />);
  await user.click(await screen.findByRole("row", { name: /Stop.*失败/ }));
  const dialog = await screen.findByRole("dialog");
  // Attempt metadata (job timeline).
  expect(dialog).toHaveTextContent("HTTP 500");
  expect(dialog).toHaveTextContent("network.timeout");
  // Only the hash fingerprint is shown — never a resolved path.
  expect(dialog).toHaveTextContent("sha256:9f2cabc123");
  expect(dialog).not.toHaveTextContent("/Users/");
  // Focus moves into the drawer; closing returns it to the initiating row.
  await user.click(screen.getByRole("button", { name: "关闭" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByRole("row", { name: /Stop.*失败/ })).toHaveFocus();
});

test("retry asks for confirmation, then reports the response", async () => {
  const backend = historyWithFailedExpiredAndSucceeded();
  const user = userEvent.setup();
  render(<HistoryPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "重试失败任务" }));
  expect(screen.getByRole("dialog", { name: "确认手动重试" })).toBeVisible();
  // Focus moves to the confirm button when the dialog opens.
  expect(screen.getByRole("button", { name: "确认重试" })).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "确认重试" }));
  await waitFor(() =>
    expect(backend.manualRetryDelivery).toHaveBeenCalledWith({ job_id: "job-evt-failed" }),
  );
  expect(await screen.findByText("重试已加入队列。")).toBeVisible();
});

test("a rejected retry surfaces the AppError message and suggested action", async () => {
  const backend = historyWithFailedExpiredAndSucceeded({
    retryError: {
      code: "delivery.retry_not_allowed",
      message: "该任务当前不可重试。",
      suggested_action: "请等待下一次自动重试。",
    },
  });
  const user = userEvent.setup();
  render(<HistoryPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "重试失败任务" }));
  await user.click(screen.getByRole("button", { name: "确认重试" }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("该任务当前不可重试。");
  expect(alert).toHaveTextContent("请等待下一次自动重试。");
});

test("empty history shows an empty state; load failure shows an alert", async () => {
  const { unmount } = render(<HistoryPage backend={historyBackend([])} />);
  expect(await screen.findByText("暂无通知历史")).toBeVisible();
  unmount();

  const failing = configuredBackend({
    channels: [testChannelSummary()],
    projects: [],
    historyListError: { code: "storage.unavailable", message: "历史库暂不可用。" },
  });
  render(<HistoryPage backend={failing} />);
  expect(await screen.findByRole("alert")).toHaveTextContent(/历史库暂不可用|列表加载失败/);
});

test("core://history-changed refreshes through a polite live region", async () => {
  const backend = historyBackend([stopFailure()]);
  render(<HistoryPage backend={backend} />);
  await screen.findByRole("table");
  act(() => {
    backend.emit("core://history-changed", { revision: 3 });
  });
  await waitFor(() => expect(backend.listHistory).toHaveBeenCalledTimes(2));
  expect(screen.getByRole("status")).toHaveTextContent("通知历史已更新");
});

test("a stale response cannot overwrite newer filter results", async () => {
  const allRows = [mkItem("evt-all"), stopFailure()];
  const backend = historyBackend(allRows);
  // Manually gated listHistory: call order decides which resolution lands.
  const gates: Array<(page: HistoryPageDto) => void> = [];
  let call = 0;
  Object.defineProperty(backend, "listHistory", {
    value: vi.fn((_input?: ListHistoryInput): Promise<HistoryPageDto> => {
      void _input;
      return new Promise((resolve) => {
        gates[call++] = resolve;
      });
    }),
  });
  const user = userEvent.setup();
  render(<HistoryPage backend={backend} />);
  // Switch 结果 to "failed" while the unfiltered page-1 request is in flight.
  await user.selectOptions(screen.getByLabelText("结果"), "failed");
  // The newer (failed) query resolves first…
  act(() => {
    gates[1]!({ items: [], next_offset: null });
  });
  await waitFor(() => expect(screen.getByText("暂无通知历史")).toBeVisible());
  // …then the stale "all" response lands last and must be discarded.
  act(() => {
    gates[0]!({ items: allRows, next_offset: null });
  });
  expect(screen.getByText("暂无通知历史")).toBeVisible();
  expect(screen.queryByRole("row", { name: /Stop/ })).not.toBeInTheDocument();
});

test("the drawer ignores a late response for a previously opened event", async () => {
  const slowItem = stopFailure(); // source_event Stop, body [REDACTED]
  const fastItem = mkItem("evt-fast", {
    source_event: "SessionStart",
    document: {
      title: "快速详情",
      severity: "info",
      facts: [],
      body: "fast body",
      footer: null,
    },
  });
  const backend = historyBackend([slowItem, fastItem]);
  const resolvers = new Map<string, (page: HistoryPageDto) => void>();
  Object.defineProperty(backend, "getHistoryDetail", {
    value: vi.fn(
      (input: GetHistoryDetailInput): Promise<HistoryPageDto> =>
        new Promise((resolve) => {
          resolvers.set(input.event_id, resolve);
        }),
    ),
  });
  const user = userEvent.setup();
  render(<HistoryPage backend={backend} />);
  // Open the slow event, then quickly switch the drawer to the fast one.
  await user.click(await screen.findByRole("row", { name: /Stop.*失败/ }));
  await user.click(await screen.findByRole("row", { name: /SessionStart/ }));
  act(() => {
    resolvers.get("evt-fast")?.({ items: [fastItem], next_offset: null });
  });
  const dialog = await screen.findByRole("dialog");
  await waitFor(() => expect(dialog).toHaveTextContent("快速详情"));
  // The slow event's late response must not replace B's drawer body.
  act(() => {
    resolvers.get("evt-stop")?.({ items: [slowItem], next_offset: null });
  });
  expect(dialog).toHaveTextContent("快速详情");
  expect(dialog).not.toHaveTextContent("[REDACTED]");
});
