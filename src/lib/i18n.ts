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
  navLabel: string;
  onboardingDetect: string;
  onboardingInstall: string;
  onboardingChannel: string;
  onboardingDefaults: string;
  onboardingTest: string;
  onboardingSteps: string;
  next: string;
  installHook: string;
  detectedAgents: string;
  detectFailed: string;
  trustPending: string;
  trustCommand: string;
  recheck: string;
  copyCommand: string;
  channelName: string;
  channelKind: string;
  kindWeCom: string;
  kindDingTalk: string;
  webhookUrl: string;
  saveChannel: string;
  useDefaults: string;
  selectChannel: string;
  sendTest: string;
  // Hook Rules page (Task 17)
  scopeLabel: string;
  scopeGlobal: string;
  scopeProject: string;
  searchHook: string;
  clearSearch: string;
  phaseLabel: string;
  enabledFilterLabel: string;
  sensitivityFilterLabel: string;
  filterAll: string;
  enabledOn: string;
  enabledOff: string;
  colSwitch: string;
  colAgent: string;
  colPhase: string;
  colFrequency: string;
  colChannels: string;
  colSource: string;
  colStatus: string;
  highFrequency: string;
  normalFrequency: string;
  experimentalBadge: string;
  deprecatedBadge: string;
  unsupportedVersion: string;
  sourceGlobal: string;
  sourceInherited: string;
  sourceOverridden: string;
  emptyRules: string;
  driftHint: string;
  applyHookChanges: string;
  confirmApplyTitle: string;
  applyAdded: string;
  applyRemoved: string;
  codexReviewWarn: string;
  cancel: string;
  confirmApply: string;
  switchRowPrefix: string;
  agentClaudeCode: string;
  agentCodex: string;
  severityPublic: string;
  severitySensitive: string;
  severityForbidden: string;
  // Drawer (Task 17)
  drawerClose: string;
  enableNotify: string;
  sectionEnabled: string;
  sectionTargets: string;
  sectionFilters: string;
  sectionPrivacy: string;
  sectionDelivery: string;
  sectionQuietHours: string;
  resetInheritedPrefix: string;
  resetInheritedSuffix: string;
  channelTemplate: string;
  filterTools: string;
  filterSubtypes: string;
  filterModes: string;
  filterModels: string;
  filterStatuses: string;
  allowedFields: string;
  maxBodyChars: string;
  summaryMode: string;
  metadataOnly: string;
  nativeSummary: string;
  extraPatterns: string;
  patternTooLong: string;
  deliveryMode: string;
  immediate: string;
  aggregate: string;
  aggregateWindow: string;
  cooldown: string;
  windowCap: string;
  statWindow: string;
  ttl: string;
  maxAttempts: string;
  quietBehavior: string;
  suppress: string;
  defer: string;
  aggregateDisabledTooltip: string;
  quietEnable: string;
  quietStart: string;
  quietEnd: string;
  quietWeekdays: string;
  bypassSeverity: string;
  bypassNone: string;
  previewTitle: string;
  sendTestAction: string;
  sendConfirmTitle: string;
  confirmSend: string;
  sentOk: string;
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
  navLabel: "主导航",
  onboardingDetect: "检测 Agent",
  onboardingInstall: "安装 Hooks",
  onboardingChannel: "添加渠道",
  onboardingDefaults: "选择默认规则",
  onboardingTest: "发送测试",
  onboardingSteps: "设置步骤",
  next: "下一步",
  installHook: "安装 Hook",
  detectedAgents: "检测结果",
  detectFailed: "检测结果获取失败，请重试。",
  trustPending: "Codex 需要确认 Hook：请运行官方命令后重新检测。",
  trustCommand: "/hooks",
  recheck: "重新检测",
  copyCommand: "复制命令",
  channelName: "渠道名称",
  channelKind: "渠道类型",
  kindWeCom: "企业微信",
  kindDingTalk: "钉钉",
  webhookUrl: "Webhook 地址",
  saveChannel: "保存渠道",
  useDefaults: "使用默认规则",
  selectChannel: "选择渠道",
  sendTest: "发送测试",
  scopeLabel: "作用域",
  scopeGlobal: "全局",
  scopeProject: "项目",
  searchHook: "搜索 Hook",
  clearSearch: "清除搜索",
  phaseLabel: "阶段",
  enabledFilterLabel: "启用状态",
  sensitivityFilterLabel: "敏感级别",
  filterAll: "全部",
  enabledOn: "已启用",
  enabledOff: "已停用",
  colSwitch: "开关",
  colAgent: "Agent",
  colPhase: "阶段",
  colFrequency: "频率",
  colChannels: "渠道",
  colSource: "配置来源",
  colStatus: "状态",
  highFrequency: "高频",
  normalFrequency: "常规",
  experimentalBadge: "实验",
  deprecatedBadge: "废弃",
  unsupportedVersion: "当前版本不支持",
  sourceGlobal: "全局",
  sourceInherited: "继承全局",
  sourceOverridden: "已覆盖",
  emptyRules: "暂无匹配的 Hook 规则",
  driftHint: "Hook 配置与已安装的 Hook 不一致。",
  applyHookChanges: "应用 Hook 变更",
  confirmApplyTitle: "确认应用 Hook 变更",
  applyAdded: "将新增",
  applyRemoved: "将移除",
  codexReviewWarn: "Codex 的变更将在应用后回到 /hooks 等待确认。",
  cancel: "取消",
  confirmApply: "确认应用 Hook 变更",
  switchRowPrefix: "切换",
  agentClaudeCode: "Claude Code",
  agentCodex: "Codex",
  severityPublic: "公开",
  severitySensitive: "敏感",
  severityForbidden: "禁止",
  drawerClose: "关闭",
  enableNotify: "启用通知",
  sectionEnabled: "启用",
  sectionTargets: "目标渠道",
  sectionFilters: "过滤条件",
  sectionPrivacy: "隐私",
  sectionDelivery: "发送策略",
  sectionQuietHours: "静默时段",
  resetInheritedPrefix: "恢复",
  resetInheritedSuffix: "继承",
  channelTemplate: "模板",
  filterTools: "工具名",
  filterSubtypes: "事件子类型",
  filterModes: "权限模式",
  filterModels: "模型",
  filterStatuses: "状态",
  allowedFields: "可外发字段",
  maxBodyChars: "正文截断上限",
  summaryMode: "摘要方式",
  metadataOnly: "仅元数据",
  nativeSummary: "原生摘要",
  extraPatterns: "自定义脱敏规则",
  patternTooLong: "自定义脱敏规则过长（每条不超过 512 字符）。",
  deliveryMode: "发送方式",
  immediate: "即时",
  aggregate: "聚合",
  aggregateWindow: "聚合窗口（秒）",
  cooldown: "冷却（秒）",
  windowCap: "窗口内上限",
  statWindow: "统计窗口（秒）",
  ttl: "有效期（秒）",
  maxAttempts: "最大尝试次数",
  quietBehavior: "静默时行为",
  suppress: "抑制",
  defer: "延后",
  aggregateDisabledTooltip: "权限请求需要即时送达，聚合不可用。",
  quietEnable: "启用静默",
  quietStart: "开始时间",
  quietEnd: "结束时间",
  quietWeekdays: "生效日",
  bypassSeverity: "高于此严重度绕过",
  bypassNone: "无",
  previewTitle: "实时预览（脱敏后）",
  sendTestAction: "发送测试",
  sendConfirmTitle: "确认发送测试到",
  confirmSend: "确认发送",
  sentOk: "测试已发送。",
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
  navLabel: "Navigation",
  onboardingDetect: "Detect Agents",
  onboardingInstall: "Install Hooks",
  onboardingChannel: "Add Channel",
  onboardingDefaults: "Choose Default Rules",
  onboardingTest: "Send Test",
  onboardingSteps: "Setup steps",
  next: "Next",
  installHook: "Install Hook",
  detectedAgents: "Detection results",
  detectFailed: "Detection failed. Please retry.",
  trustPending:
    "Codex needs hook confirmation: run the official command, then re-check.",
  trustCommand: "/hooks",
  recheck: "Recheck",
  copyCommand: "Copy command",
  channelName: "Channel name",
  channelKind: "Channel kind",
  kindWeCom: "WeCom",
  kindDingTalk: "DingTalk",
  webhookUrl: "Webhook URL",
  saveChannel: "Save channel",
  useDefaults: "Use default rules",
  selectChannel: "Select channel",
  sendTest: "Send test",
  scopeLabel: "Scope",
  scopeGlobal: "Global",
  scopeProject: "Project",
  searchHook: "Search hooks",
  clearSearch: "Clear search",
  phaseLabel: "Phase",
  enabledFilterLabel: "Enabled",
  sensitivityFilterLabel: "Sensitivity",
  filterAll: "All",
  enabledOn: "Enabled",
  enabledOff: "Disabled",
  colSwitch: "Switch",
  colAgent: "Agent",
  colPhase: "Phase",
  colFrequency: "Rate",
  colChannels: "Channels",
  colSource: "Source",
  colStatus: "Status",
  highFrequency: "High-freq",
  normalFrequency: "Normal",
  experimentalBadge: "Experimental",
  deprecatedBadge: "Deprecated",
  unsupportedVersion: "Not supported in this version",
  sourceGlobal: "Global",
  sourceInherited: "Inherited from global",
  sourceOverridden: "Overridden",
  emptyRules: "No matching hook rules",
  driftHint: "Rule selection differs from the installed hooks.",
  applyHookChanges: "Apply hook changes",
  confirmApplyTitle: "Confirm applying hook changes",
  applyAdded: "To add",
  applyRemoved: "To remove",
  codexReviewWarn:
    "Codex changes return to /hooks for review after applying.",
  cancel: "Cancel",
  confirmApply: "Confirm applying hook changes",
  switchRowPrefix: "Toggle",
  agentClaudeCode: "Claude Code",
  agentCodex: "Codex",
  severityPublic: "Public",
  severitySensitive: "Sensitive",
  severityForbidden: "Forbidden",
  drawerClose: "Close",
  enableNotify: "Enable notifications",
  sectionEnabled: "Enabled",
  sectionTargets: "Target channels",
  sectionFilters: "Filters",
  sectionPrivacy: "Privacy",
  sectionDelivery: "Delivery policy",
  sectionQuietHours: "Quiet hours",
  resetInheritedPrefix: "Reset ",
  resetInheritedSuffix: " to inherited",
  channelTemplate: "Template",
  filterTools: "Tool names",
  filterSubtypes: "Event subtypes",
  filterModes: "Permission modes",
  filterModels: "Models",
  filterStatuses: "Statuses",
  allowedFields: "Exportable fields",
  maxBodyChars: "Body truncation limit",
  summaryMode: "Summary mode",
  metadataOnly: "Metadata only",
  nativeSummary: "Native summary",
  extraPatterns: "Custom redaction patterns",
  patternTooLong: "Custom redaction pattern is too long (512 characters max each).",
  deliveryMode: "Delivery mode",
  immediate: "Immediate",
  aggregate: "Aggregate",
  aggregateWindow: "Aggregate window (s)",
  cooldown: "Cooldown (s)",
  windowCap: "Max per window",
  statWindow: "Statistics window (s)",
  ttl: "TTL (s)",
  maxAttempts: "Max attempts",
  quietBehavior: "During quiet hours",
  suppress: "Suppress",
  defer: "Defer",
  aggregateDisabledTooltip:
    "Permission requests must deliver immediately; aggregation is unavailable.",
  quietEnable: "Enable quiet hours",
  quietStart: "Start time",
  quietEnd: "End time",
  quietWeekdays: "Active weekdays",
  bypassSeverity: "Bypass at or above severity",
  bypassNone: "None",
  previewTitle: "Live preview (redacted)",
  sendTestAction: "Send test",
  sendConfirmTitle: "Confirm test send to",
  confirmSend: "Send now",
  sentOk: "Test sent.",
};

export function dictionary(locale: LocaleCode): Dictionary {
  return locale === "en" ? en : zhCn;
}
