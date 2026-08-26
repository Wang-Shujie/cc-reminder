import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

import { BackendProvider } from "../lib/backend";
import { configuredBackend, testChannelSummary } from "../test/TestApp";
import { IntegrationsPage } from "./IntegrationsPage";

test("destinations keep the full channel table; add jumps to settings", async () => {
  const onNavigate = vi.fn();
  const user = userEvent.setup();
  render(
    <BackendProvider
      backend={configuredBackend({ channels: [testChannelSummary()] })}
    >
      <IntegrationsPage locale="zh_cn" onNavigate={onNavigate} />
    </BackendProvider>,
  );
  expect(await screen.findByRole("heading", { name: "通知来源" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Agent 集成" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "通知去向" })).toBeVisible();
  // 图1形态:完整渠道统计表(名称/凭据/状态/上次成功/操作)。
  expect(await screen.findByRole("cell", { name: "值班群" })).toBeVisible();
  expect(screen.getByRole("columnheader", { name: "上次成功" })).toBeVisible();
  expect(screen.getByRole("button", { name: "测试发送" })).toBeVisible();
  await user.click(screen.getByRole("button", { name: "添加渠道" }));
  expect(onNavigate).toHaveBeenCalledWith("settings");
});
