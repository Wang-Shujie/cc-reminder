import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { BackendProvider } from "../../lib/backend";
import { configuredBackend, type FakeBackend } from "../../test/TestApp";
import type { ChannelId, HistoryItem } from "../../lib/contracts";
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

test("overview and the bottom log pane render as one page", async () => {
  renderWorkbench(configuredBackend());
  expect(await screen.findByRole("heading", { name: "概览" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "通知历史" })).toBeVisible();
});

test("失败 preset refilters the log pane and keeps both sections present", async () => {
  const user = userEvent.setup();
  renderWorkbench(
    configuredBackend({
      historyItems: [
        item({}),
        item({
          event_id: "evt-2",
          source_event: "StopSuccess",
          delivery_status: "succeeded",
          delivery_job_id: "job-2",
        }),
      ],
    }),
  );
  // 预设筛选(用户裁决):失败预设承担原"查看失败任务"的职能。
  await user.click(await screen.findByRole("button", { name: "失败" }));
  const failedRows = await screen.findAllByText("PostToolUseFailure");
  expect(failedRows.length).toBeGreaterThan(0);
  expect(screen.queryByText("StopSuccess")).not.toBeInTheDocument();
  // 概览区仍在同一页。
  expect(screen.getByRole("heading", { name: "概览" })).toBeVisible();
});
