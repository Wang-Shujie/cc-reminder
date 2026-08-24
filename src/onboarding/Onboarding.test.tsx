import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  TestApp,
  backendNeedingCodexTrust,
  onboardingBackend,
  type FakeBackend,
} from "../test/TestApp";

async function completeDetectAndInstall(user: ReturnType<typeof userEvent.setup>) {
  // Detection runs on mount; wait for its results before advancing.
  expect(await screen.findByText("claude-code")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "下一步" }));
  await user.click(await screen.findByRole("button", { name: "安装 Hook" }));
}

async function completeChannelAndDefaults(user: ReturnType<typeof userEvent.setup>) {
  await screen.findByRole("heading", { name: "添加渠道" });
  await user.type(screen.getByLabelText("渠道名称"), "工程群");
  await user.type(
    screen.getByLabelText("Webhook 地址"),
    "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=abc123",
  );
  await user.click(screen.getByRole("button", { name: "保存渠道" }));
  await screen.findByRole("heading", { name: "选择默认规则" });
  await user.click(screen.getByRole("button", { name: "使用默认规则" }));
}

test("onboarding follows detect install channel defaults test order", async () => {
  const user = userEvent.setup();
  render(<TestApp backend={onboardingBackend()} />);
  expect(await screen.findByRole("heading", { name: "检测 Agent" })).toBeVisible();
  await completeDetectAndInstall(user);
  await completeChannelAndDefaults(user);
  expect(await screen.findByRole("heading", { name: "发送测试" })).toBeVisible();
});

test("Codex trust is a separate blocking checklist item with official command", async () => {
  render(<TestApp backend={backendNeedingCodexTrust()} />);
  expect(await screen.findByText("/hooks")).toBeVisible();
  expect(screen.getByRole("button", { name: "重新检测" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "复制命令" })).toBeInTheDocument();
  expect(screen.queryByText(/bypass/i)).not.toBeInTheDocument();
  // The trust item blocks advancing to hook installation.
  expect(screen.getByRole("button", { name: "下一步" })).toBeDisabled();
});

test("completion is persisted only after a successful test send", async () => {
  const user = userEvent.setup();
  const backend = onboardingBackend();
  render(<TestApp backend={backend} />);
  await completeDetectAndInstall(user);
  await completeChannelAndDefaults(user);
  await screen.findByRole("heading", { name: "发送测试" });
  expect(backend.saveSettings).not.toHaveBeenCalled();
  const channelSelect = await screen.findByLabelText("选择渠道");
  expect((channelSelect as HTMLSelectElement).value).not.toBe("");
  await user.click(screen.getByRole("button", { name: "发送测试" }));
  await waitFor(() =>
    expect(backend.saveSettings).toHaveBeenCalledWith(
      expect.objectContaining({ onboarding_completed: true }),
    ),
  );
  // Completion swaps the onboarding for the shell at Hook Rules.
  expect(await screen.findByRole("heading", { name: "Hook 规则" })).toBeVisible();
});

test("onboarding resumes at the first incomplete step", async () => {
  // A channel already exists, so detect/install/channel are done: the flow
  // resumes at "choose default rules" instead of restarting from detection.
  render(
    <TestApp
      backend={onboardingBackend({
        channels: [
          {
            id: "channel-1",
            kind: "we_com",
            name: "已有渠道",
            credential_present: true,
            health: "unknown",
            paused: false,
            last_succeeded_at: null,
          },
        ],
      })}
    />,
  );
  expect(await screen.findByRole("heading", { name: "选择默认规则" })).toBeVisible();
  expect(screen.queryByRole("heading", { name: "检测 Agent" })).not.toBeInTheDocument();
});
