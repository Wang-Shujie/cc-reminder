// Task 18 contract tests for the Channels page. The plan's Step 2 block is
// authoritative; surrounding assertions lock credential hygiene, host
// validation, delete-blocking, paused-auth state and test-send receipts.
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { configuredBackend, type FakeBackend } from "../../test/TestApp";
import { ChannelsPage } from "./ChannelsPage";

import type { ChannelId } from "../../lib/contracts";

/** A saved DingTalk channel whose webhook (with access_token=fake) went
 *  through the real write path; the read model must never echo it. */
async function savedDingTalkBackend(): Promise<FakeBackend> {
  const backend = configuredBackend();
  await backend.saveChannel({
    channel_id: null,
    name: "钉钉值班群",
    keyword_prefix: "CC",
    credential: {
      kind: "ding_talk",
      webhook: "https://oapi.dingtalk.com/robot/send?access_token=fake",
    },
  });
  return backend;
}

async function savedWeComBackend(options?: Parameters<typeof configuredBackend>[0]): Promise<{
  backend: FakeBackend;
  channelId: ChannelId;
}> {
  const backend = configuredBackend(options);
  const saved = await backend.saveChannel({
    channel_id: null,
    name: "企业微信群",
    credential: {
      kind: "we_com",
      webhook: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake2",
    },
  });
  return { backend, channelId: saved.id };
}

test("saved credentials are never placed into an input or DOM", async () => {
  const backend = await savedDingTalkBackend();
  render(<ChannelsPage backend={backend} />);
  expect(await screen.findByText("已保存凭据")).toBeVisible();
  expect(screen.getByLabelText("Webhook")).toHaveValue("");
  expect(document.body.textContent).not.toContain("access_token=fake");
  // Out-of-scope controls stay out of scope: no @all or phone-list anywhere.
  expect(screen.queryByText(/@所有人/)).toBeNull();
});

test("connection test requires confirmation because it sends a real group message", async () => {
  const user = userEvent.setup();
  const { backend, channelId } = await savedWeComBackend();
  render(<ChannelsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "测试发送" }));
  expect(screen.getByRole("dialog")).toHaveTextContent("将向目标群发送测试消息");

  await user.click(screen.getByRole("button", { name: "确认发送" }));
  await waitFor(() =>
    expect(backend.testChannel).toHaveBeenCalledWith({ channel_id: channelId }),
  );
  // Receipt display includes the HTTP status.
  expect(await screen.findByText(/HTTP 200/)).toBeVisible();
});

test("DingTalk form exposes optional signing secret and keyword prefix and passes them through", async () => {
  const user = userEvent.setup();
  const backend = configuredBackend();
  render(<ChannelsPage backend={backend} />);
  await screen.findByLabelText("Webhook");

  await user.selectOptions(screen.getByLabelText("渠道类型"), "ding_talk");
  const secret = screen.getByLabelText("签名密钥（可选）");
  const prefix = screen.getByLabelText("关键词前缀");
  expect(secret).toHaveValue("");
  await user.type(screen.getByLabelText("渠道名称"), "值班群");
  await user.type(screen.getByLabelText("Webhook"), "https://oapi.dingtalk.com/robot/send?access_token=new");
  await user.type(secret, "SEC123");
  await user.type(prefix, "CC");
  await user.click(screen.getByRole("button", { name: "保存渠道" }));

  await waitFor(() =>
    expect(backend.saveChannel).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "值班群",
        keyword_prefix: "CC",
        credential: {
          kind: "ding_talk",
          webhook: "https://oapi.dingtalk.com/robot/send?access_token=new",
          signing_secret: "SEC123",
        },
      }),
    ),
  );
});

test("unofficial hosts surface the backend validation error without storing anything", async () => {
  const user = userEvent.setup();
  const backend = configuredBackend();
  render(<ChannelsPage backend={backend} />);
  await screen.findByLabelText("Webhook");
  await user.type(screen.getByLabelText("渠道名称"), "坏地址");
  await user.type(screen.getByLabelText("Webhook"), "https://example.com/hook");
  await user.click(screen.getByRole("button", { name: "保存渠道" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("webhook rejected");
  expect(backend.saveChannel).toHaveBeenCalledTimes(1); // attempted once, refused
  expect(screen.queryByText("坏地址")).not.toBeInTheDocument(); // never listed
});

test("deleting a channel targeted by rules surfaces the backend refusal", async () => {
  const user = userEvent.setup();
  // The fake assigns sequential ids; the first saved channel is "channel-1".
  const { backend } = await savedWeComBackend({
    channelInUseIds: ["channel-1" as ChannelId],
  });
  render(<ChannelsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: /删除渠道/ }));
  expect(screen.getByRole("dialog")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "确认删除" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("channel is targeted");
  expect(screen.getByText("企业微信群")).toBeVisible(); // still listed
});

test("paused auth state and last success time are visible per channel", async () => {
  const paused = configuredBackend({
    channels: [
      {
        id: "ch-paused" as ChannelId,
        kind: "we_com",
        name: "已暂停群",
        credential_present: true,
        health: "auth_paused",
        paused: true,
        last_succeeded_at: "2026-08-01T08:30:00Z",
      },
    ],
  });
  render(<ChannelsPage backend={paused} />);
  const row = await screen.findByRole("row", { name: /已暂停群/ });
  expect(within(row).getByText("已暂停")).toBeVisible();
  expect(within(row).getByText(/2026/)).toBeVisible();
});

test("a markdown fallback receipt is displayed when the platform reports one", async () => {
  const user = userEvent.setup();
  const { backend } = await savedWeComBackend({
    testChannelResult: { http_status: 200, platform_code: "45033" },
  });
  render(<ChannelsPage backend={backend} />);
  await user.click(await screen.findByRole("button", { name: "测试发送" }));
  await user.click(screen.getByRole("button", { name: "确认发送" }));
  expect(await screen.findByText(/Markdown/)).toBeVisible();
});

test("multiple instances each keep their own actions and edit target", async () => {
  const user = userEvent.setup();
  const backend = await savedDingTalkBackend();
  await backend.saveChannel({
    channel_id: null,
    name: "第二渠道",
    credential: {
      kind: "we_com",
      webhook: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=second",
    },
  });
  render(<ChannelsPage backend={backend} />);
  expect(await screen.findAllByRole("button", { name: "测试发送" })).toHaveLength(2);

  // Selecting the second instance for credential replacement keeps the saved
  // webhook out of the input.
  await user.click(screen.getByRole("button", { name: "替换凭据 第二渠道" }));
  expect(screen.getByLabelText("Webhook")).toHaveValue("");
  expect(document.body.textContent).not.toContain("key=second");
});

test("failed primary channel list surfaces an incomplete-data alert", async () => {
  const backend = await savedDingTalkBackend();
  backend.listChannels = async () => {
    throw { code: "internal_error", message: "boom" };
  };
  render(<ChannelsPage backend={backend} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("列表加载失败");
});
