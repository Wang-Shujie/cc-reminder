// Typed UI dictionary. zh-CN is complete and authoritative; en mirrors it.
import type { LocaleCode } from "./contracts";

export interface Dictionary {
  navOverview: string;
  navAgents: string;
  navHooks: string;
  navChannels: string;
  navProjects: string;
  navHistory: string;
  navSettings: string;
  statusTitle: string;
  pendingJobs: string;
  failedJobs: string;
  loading: string;
  pagePlaceholder: string;
  onboardingDetect: string;
  onboardingInstall: string;
  onboardingChannel: string;
  onboardingDefaults: string;
  onboardingTest: string;
  next: string;
  installHook: string;
  detectedAgents: string;
  trustPending: string;
  trustCommand: string;
  recheck: string;
  copyCommand: string;
  channelName: string;
  webhookUrl: string;
  saveChannel: string;
  useDefaults: string;
  selectChannel: string;
  sendTest: string;
}

const zhCn: Dictionary = {
  navOverview: "概览",
  navAgents: "Agent 集成",
  navHooks: "Hook 规则",
  navChannels: "渠道",
  navProjects: "项目",
  navHistory: "通知历史",
  navSettings: "设置",
  statusTitle: "CC Reminder",
  pendingJobs: "待发送",
  failedJobs: "失败任务",
  loading: "加载中…",
  pagePlaceholder: "此页面将在后续版本中提供。",
  onboardingDetect: "检测 Agent",
  onboardingInstall: "安装 Hooks",
  onboardingChannel: "添加渠道",
  onboardingDefaults: "选择默认规则",
  onboardingTest: "发送测试",
  next: "下一步",
  installHook: "安装 Hook",
  detectedAgents: "检测结果",
  trustPending: "Codex 需要确认 Hook：请运行官方命令后重新检测。",
  trustCommand: "/hooks",
  recheck: "重新检测",
  copyCommand: "复制命令",
  channelName: "渠道名称",
  webhookUrl: "Webhook 地址",
  saveChannel: "保存渠道",
  useDefaults: "使用默认规则",
  selectChannel: "选择渠道",
  sendTest: "发送测试",
};

const en: Dictionary = {
  navOverview: "Overview",
  navAgents: "Agent Integrations",
  navHooks: "Hook Rules",
  navChannels: "Channels",
  navProjects: "Projects",
  navHistory: "Notification History",
  navSettings: "Settings",
  statusTitle: "CC Reminder",
  pendingJobs: "Pending",
  failedJobs: "Failed jobs",
  loading: "Loading…",
  pagePlaceholder: "This page arrives in a later release.",
  onboardingDetect: "Detect Agents",
  onboardingInstall: "Install Hooks",
  onboardingChannel: "Add Channel",
  onboardingDefaults: "Choose Default Rules",
  onboardingTest: "Send Test",
  next: "Next",
  installHook: "Install Hook",
  detectedAgents: "Detection results",
  trustPending:
    "Codex needs hook confirmation: run the official command, then re-check.",
  trustCommand: "/hooks",
  recheck: "Recheck",
  copyCommand: "Copy command",
  channelName: "Channel name",
  webhookUrl: "Webhook URL",
  saveChannel: "Save channel",
  useDefaults: "Use default rules",
  selectChannel: "Select channel",
  sendTest: "Send test",
};

export function dictionary(locale: LocaleCode): Dictionary {
  return locale === "en" ? en : zhCn;
}
