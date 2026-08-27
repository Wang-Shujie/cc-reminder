import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ChannelGuide } from "./ChannelGuide";

test("guide defaults open with the WeCom steps and follows the platform", async () => {
  const user = userEvent.setup();
  const { rerender } = render(<ChannelGuide locale="zh_cn" kind="we_com" />);
  expect(screen.getByRole("button", { name: "如何获取 Webhook?" })).toBeVisible();
  expect(screen.getByText(/企业微信.*添加群机器人/s)).toBeVisible();
  // 切到钉钉:分步内容跟随平台。
  rerender(<ChannelGuide locale="zh_cn" kind="ding_talk" />);
  expect(screen.getByText(/智能群助手/s)).toBeVisible();
  // 折叠并记住( localStorage),再次挂载保持收起。
  await user.click(screen.getByRole("button", { name: "如何获取 Webhook?" }));
  expect(screen.queryByText(/智能群助手/s)).not.toBeInTheDocument();
  rerender(<ChannelGuide locale="zh_cn" kind="ding_talk" />);
  expect(screen.queryByText(/智能群助手/s)).not.toBeInTheDocument();
});

test("guide renders English steps for the en locale", () => {
  render(<ChannelGuide locale="en" kind="we_com" />);
  expect(screen.getByText(/right-click the group/s)).toBeVisible();
});
