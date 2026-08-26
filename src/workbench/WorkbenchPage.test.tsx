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
