import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  TestApp,
  backendNeedingCodexTrust,
  onboardingBackend,
  type FakeBackend,
} from "../test/TestApp";
import type {
  AgentIntegrationSummary,
  ChannelSummary,
} from "../lib/contracts";

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
    <TestApp backend={onboardingBackend({ channels: [existingChannel()] })} />,
  );
  expect(await screen.findByRole("heading", { name: "选择默认规则" })).toBeVisible();
  expect(screen.queryByRole("heading", { name: "检测 Agent" })).not.toBeInTheDocument();
});

function existingChannel(): ChannelSummary {
  return {
    id: "channel-1",
    kind: "we_com",
    name: "已有渠道",
    credential_present: true,
    health: "unknown",
    paused: false,
    last_succeeded_at: null,
  };
}

function detectionResults(codexNeedsConfirmation: boolean): AgentIntegrationSummary[] {
  return [
    {
      agent: "claude-code",
      installed: true,
      version: "2.1.218",
      executable_path: "/usr/local/bin/claude",
      health: "detected",
      needs_compatible_version_confirmation: false,
    },
    {
      agent: "codex",
      installed: true,
      version: "0.145.0",
      executable_path: "/usr/local/bin/codex",
      health: "detected",
      needs_compatible_version_confirmation: codexNeedsConfirmation,
    },
  ];
}

test("resume with a channel still blocks on a pending Codex trust confirmation", async () => {
  const user = userEvent.setup();
  let codexNeedsConfirmation = true;
  const backend = onboardingBackend({
    channels: [existingChannel()],
    detectResults: () => detectionResults(codexNeedsConfirmation),
  });
  render(<TestApp backend={backend} />);
  // The trust checklist is shown even though a channel already exists; the
  // flow must NOT jump straight to the defaults step.
  expect(await screen.findByText("/hooks")).toBeVisible();
  expect(screen.getByRole("button", { name: "下一步" })).toBeDisabled();
  expect(
    screen.queryByRole("heading", { name: "选择默认规则" }),
  ).not.toBeInTheDocument();
  // A recheck that comes back clean clears the gate and resumes to defaults.
  codexNeedsConfirmation = false;
  await user.click(screen.getByRole("button", { name: "重新检测" }));
  expect(
    await screen.findByRole("heading", { name: "选择默认规则" }),
  ).toBeVisible();
});

test("a failed detection shows an alert with retry and keeps Next disabled", async () => {
  const user = userEvent.setup();
  const backend = onboardingBackend();
  vi.mocked(backend.detectAgents).mockRejectedValueOnce(new Error("detect down"));
  render(<TestApp backend={backend} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("检测结果获取失败");
  expect(screen.getByRole("button", { name: "下一步" })).toBeDisabled();
  // Retrying without the injected failure recovers the normal detect list.
  await user.click(screen.getByRole("button", { name: "重新检测" }));
  expect(await screen.findByText("codex")).toBeVisible();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "下一步" })).toBeEnabled();
});

test("a failed channel save surfaces an error instead of advancing", async () => {
  const user = userEvent.setup();
  const backend = onboardingBackend();
  vi.spyOn(backend, "saveChannel").mockRejectedValue(new Error("webhook rejected"));
  render(<TestApp backend={backend} />);
  await completeDetectAndInstall(user);
  await screen.findByRole("heading", { name: "添加渠道" });
  await user.type(screen.getByLabelText("渠道名称"), "工程群");
  await user.type(
    screen.getByLabelText("Webhook 地址"),
    "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=abc123",
  );
  await user.click(screen.getByRole("button", { name: "保存渠道" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("webhook rejected");
  expect(
    screen.queryByRole("heading", { name: "选择默认规则" }),
  ).not.toBeInTheDocument();
});

test("ding_talk channels expose signing secret and keyword prefix and save them", async () => {
  const user = userEvent.setup();
  const backend = onboardingBackend();
  const saveSpy = vi.spyOn(backend, "saveChannel");
  render(<TestApp backend={backend} />);
  await completeDetectAndInstall(user);
  await screen.findByRole("heading", { name: "添加渠道" });
  // WeCom (default): the DingTalk-only fields stay hidden.
  expect(screen.queryByLabelText("签名密钥（可选）")).not.toBeInTheDocument();
  expect(screen.queryByLabelText("关键词前缀")).not.toBeInTheDocument();
  await user.selectOptions(screen.getByLabelText("渠道类型"), "ding_talk");
  const secret = screen.getByLabelText("签名密钥（可选）");
  const prefix = screen.getByLabelText("关键词前缀");
  expect(secret).toBeVisible();
  expect(prefix).toBeVisible();
  await user.type(secret, "SECabcdef123");
  await user.type(prefix, "CC Reminder");
  await user.type(
    screen.getByLabelText("Webhook 地址"),
    "https://oapi.dingtalk.com/robot/send?access_token=tok123",
  );
  await user.click(screen.getByRole("button", { name: "保存渠道" }));
  expect(await screen.findByRole("heading", { name: "选择默认规则" })).toBeVisible();
  expect(saveSpy).toHaveBeenCalledWith(
    expect.objectContaining({
      keyword_prefix: "CC Reminder",
      credential: {
        kind: "ding_talk",
        webhook: "https://oapi.dingtalk.com/robot/send?access_token=tok123",
        signing_secret: "SECabcdef123",
      },
    }),
  );
  // Back navigation returns to the channel step without undoing anything.
  await user.click(screen.getByRole("button", { name: "上一步" }));
  expect(
    await screen.findByRole("heading", { name: "添加渠道" }),
  ).toBeVisible();
});
