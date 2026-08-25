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
  exportDiagnostics: string;
  clearHistory: string;
  clearHistoryWarning: string;
  confirmClearHistory: string;
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
  colHook: string;
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
  versionConsentDisclosure: string;
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
  // Agent Integration page (Task 18)
  agentDetect: string;
  agentDetecting: string;
  agentInstallPrefix: string;
  agentRepairPrefix: string;
  agentUninstallPrefix: string;
  hookWord: string;
  colVersion: string;
  colState: string;
  dsDetected: string;
  dsMissing: string;
  dsInvalidVersion: string;
  dsProcessFailed: string;
  dsTimedOut: string;
  agentUpgradeNeeded: string;
  upgradeHelperAction: string;
  uninstallConfirmTitle: string;
  uninstallScopeNote: string;
  confirmUninstall: string;
  versionConsentTitle: string;
  consentContinue: string;
  lastApplied: string;
  ehHealthy: string;
  ehMissing: string;
  ehDrifted: string;
  ehHelperMismatch: string;
  ehNeedsTrust: string;
  ehAgentUpgradeRequired: string;
  trustNotice: string;
  // Channels page (Task 18)
  addChannelAction: string;
  replaceCredentialAction: string;
  deleteChannelAction: string;
  deleteChannelConfirmTitle: string;
  deleteChannelNote: string;
  confirmDelete: string;
  webhookField: string;
  signingSecret: string;
  keywordPrefixField: string;
  savedCredentialBadge: string;
  credentialReplaceHint: string;
  testSendBtn: string;
  testSendConfirmTitle: string;
  testSendWarning: string;
  lastSuccessCol: string;
  neverSucceeded: string;
  pausedBadge: string;
  authPausedNote: string;
  platformCodeLabel: string;
  markdownFallbackNote: string;
  testResultsTitle: string;
  emptyChannels: string;
  channelColName: string;
  channelColKind: string;
  channelColCredential: string;
  channelColHealth: string;
  // Projects page (Task 18)
  addProjectBtn: string;
  projectNameField: string;
  worktreeChoiceLegend: string;
  aliasChoiceLabel: string;
  separateChoiceLabel: string;
  saveBtn: string;
  pickedPathLabel: string;
  colProject: string;
  colRoot: string;
  colAliases: string;
  colOverrides: string;
  removeAliasBtn: string;
  removeAliasConfirmTitle: string;
  removeAliasNote: string;
  confirmRemove: string;
  selectAgent: string;
  allAgentsOption: string;
  noProjects: string;
  scanBoundaryNote: string;
  listLoadFailed: string;
  listSeparator: string;
  // Overview page (Task 19)
  metricPending: string;
  metricRetry: string;
  metricFailed: string;
  metricExpired: string;
  metricSpool: string;
  metricRejected: string;
  lastSuccessLabel: string;
  viewFailedJobs: string;
  overviewIssues: string;
  noIssues: string;
  recentFailuresTitle: string;
  noRecentFailures: string;
  recentFailuresLoadFailed: string;
  gotoAgents: string;
  gotoChannels: string;
  gotoHistory: string;
  overviewLoadFailed: string;
  refreshedNotice: string;
  // History page (Task 19)
  filterResult: string;
  filterTimeFrom: string;
  filterTimeUntil: string;
  stNotQueued: string;
  stPending: string;
  stSending: string;
  stRetryWait: string;
  stSucceeded: string;
  stFailed: string;
  stExpired: string;
  colTime: string;
  retryFailedJob: string;
  retryExpiredJob: string;
  retryExpiredHint: string;
  retryConfirmTitle: string;
  retryConfirmNote: string;
  confirmRetry: string;
  retryQueued: string;
  loadMore: string;
  emptyHistory: string;
  detailTitle: string;
  detailOccurred: string;
  detailReceived: string;
  detailModel: string;
  detailMode: string;
  detailUnmatchedFingerprint: string;
  detailAttempts: string;
  detailNoDocument: string;
  historyUpdated: string;
  unmatchedProject: string;
  // Settings page (Task 19)
  sectionStartup: string;
  autostartLabel: string;
  closeToTrayLabel: string;
  languageLabel: string;
  langZh: string;
  langEn: string;
  localeRestartHint: string;
  themeLabel: string;
  themeSystem: string;
  themeLight: string;
  themeDark: string;
  retentionSection: string;
  eventRetentionLabel: string;
  logRetentionLabel: string;
  retentionBounds: string;
  savedOk: string;
  settingsLoadFailed: string;
  pauseSection: string;
  pausedUntilPrefix: string;
  notPaused: string;
  pause15m: string;
  pause1h: string;
  pauseToday: string;
  resumeNotifications: string;
  updatesSection: string;
  checkUpdates: string;
  checkingUpdates: string;
  upToDate: string;
  updateAvailablePrefix: string;
  installUpdateAction: string;
  installConfirmTitle: string;
  confirmInstall: string;
  updateInstalled: string;
  credentialStoreSection: string;
  // Task 20 fix round 1: debug logging window + export result disclosure.
  debugSection: string;
  debugLoggingLabel: string;
  debugOff: string;
  debug15m: string;
  debug60m: string;
  diagnosticsSavedPrefix: string;
  diagnosticsCancelled: string;
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
  exportDiagnostics: "导出诊断",
  clearHistory: "清除历史",
  clearHistoryWarning: "将删除已完成的通知历史；活动任务及其事件将被保留。",
  confirmClearHistory: "确认清除历史",
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
  colHook: "Hook",
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
  versionConsentDisclosure: "检测到的 Agent 版本尚未经精确验证，确认后将继续安装。",
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
  // Preview reflects the SAVED rule; unsaved edits are deliberately excluded.
  previewTitle: "已保存配置的预览（脱敏后）",
  sendTestAction: "发送测试",
  sendConfirmTitle: "确认发送测试到",
  confirmSend: "确认发送",
  sentOk: "测试已发送。",
  // Agent Integration page (Task 18)
  agentDetect: "检测 Agent",
  agentDetecting: "检测中…",
  agentInstallPrefix: "安装 ",
  agentRepairPrefix: "修复 ",
  agentUninstallPrefix: "卸载 ",
  hookWord: " Hook",
  colVersion: "版本",
  colState: "状态",
  dsDetected: "已检测",
  dsMissing: "未检测到",
  dsInvalidVersion: "版本无效",
  dsProcessFailed: "检测进程失败",
  dsTimedOut: "检测超时",
  agentUpgradeNeeded: "需要升级 CC Reminder",
  upgradeHelperAction: "升级 Helper",
  uninstallConfirmTitle: "确认卸载 Hook",
  uninstallScopeNote: "只移除 CC Reminder 创建的 Hook，Agent 自身的其他 Hook 保持不变。",
  confirmUninstall: "确认卸载",
  versionConsentTitle: "确认在未验证版本上继续",
  consentContinue: "确认继续",
  lastApplied: "最近应用结果",
  ehHealthy: "健康",
  ehMissing: "缺失",
  ehDrifted: "不一致",
  ehHelperMismatch: "Helper 版本不匹配",
  ehNeedsTrust: "待确认",
  ehAgentUpgradeRequired: "需要升级 Agent",
  trustNotice: "Codex 的 Hook 需要在官方界面确认：请运行以下命令后重新检测。",
  // Channels page (Task 18)
  addChannelAction: "添加渠道",
  replaceCredentialAction: "替换凭据",
  deleteChannelAction: "删除渠道",
  deleteChannelConfirmTitle: "确认删除渠道",
  deleteChannelNote: "删除后指向该渠道的规则将无法投递。",
  confirmDelete: "确认删除",
  webhookField: "Webhook",
  signingSecret: "签名密钥（可选）",
  keywordPrefixField: "关键词前缀",
  savedCredentialBadge: "已保存凭据",
  credentialReplaceHint: "已保存的凭据不会回填；输入新的 Webhook 以替换。",
  testSendBtn: "测试发送",
  testSendConfirmTitle: "确认测试发送",
  testSendWarning: "将向目标群发送测试消息。",
  lastSuccessCol: "上次成功",
  neverSucceeded: "尚未成功",
  pausedBadge: "已暂停",
  authPausedNote: "授权已暂停：请替换凭据。",
  platformCodeLabel: "平台返回码",
  markdownFallbackNote: "平台不支持 Markdown 时已自动改用纯文本发送。",
  testResultsTitle: "最近测试发送结果",
  emptyChannels: "尚未添加渠道",
  channelColName: "名称",
  channelColKind: "类型",
  channelColCredential: "凭据",
  channelColHealth: "状态",
  // Projects page (Task 18)
  addProjectBtn: "添加项目",
  projectNameField: "项目名称",
  worktreeChoiceLegend: "该目录如何参与匹配？",
  aliasChoiceLabel: "作为现有项目的路径别名",
  separateChoiceLabel: "作为独立项目添加",
  saveBtn: "保存",
  pickedPathLabel: "所选目录",
  colProject: "项目",
  colRoot: "根目录",
  colAliases: "路径别名",
  colOverrides: "覆盖规则数",
  removeAliasBtn: "移除别名",
  removeAliasConfirmTitle: "确认移除路径别名",
  removeAliasNote: "移除后该目录的事件将不再匹配此项目。",
  confirmRemove: "确认移除",
  selectAgent: "选择 Agent",
  allAgentsOption: "全部 Agent",
  noProjects: "尚未添加项目",
  scanBoundaryNote: "仅检查你明确选择的目录及其上级目录以识别 Git 根，不会扫描整个磁盘。",
  listLoadFailed: "列表加载失败，页面数据可能不完整。",
  listSeparator: "、",
  // Overview page (Task 19)
  metricPending: "{n} 个待发送任务",
  metricRetry: "{n} 个等待重试任务",
  metricFailed: "{n} 个失败任务",
  metricExpired: "{n} 个过期任务",
  metricSpool: "{n} 个暂存事件",
  metricRejected: "{n} 个被拒绝事件",
  lastSuccessLabel: "上次成功：{time}",
  viewFailedJobs: "查看失败任务",
  overviewIssues: "待处理问题",
  noIssues: "当前没有待处理的问题。",
  recentFailuresTitle: "最近失败",
  noRecentFailures: "最近没有失败任务。",
  recentFailuresLoadFailed: "失败任务列表加载失败。",
  gotoAgents: "前往 Agent 集成",
  gotoChannels: "前往渠道",
  gotoHistory: "查看通知历史",
  overviewLoadFailed: "健康状态加载失败，请稍后重试。",
  refreshedNotice: "数据已刷新。",
  // History page (Task 19)
  filterResult: "结果",
  filterTimeFrom: "时间从",
  filterTimeUntil: "时间至",
  stNotQueued: "未入队",
  stPending: "待发送",
  stSending: "发送中",
  stRetryWait: "等待重试",
  stSucceeded: "已成功",
  stFailed: "发送失败",
  stExpired: "已过期",
  colTime: "时间",
  retryFailedJob: "重试失败任务",
  retryExpiredJob: "重试过期任务",
  retryExpiredHint: "过期任务不可重试。",
  retryConfirmTitle: "确认手动重试",
  retryConfirmNote: "该任务将立即重新投递到原渠道。",
  confirmRetry: "确认重试",
  retryQueued: "重试已加入队列。",
  loadMore: "加载更多",
  emptyHistory: "暂无通知历史",
  detailTitle: "通知详情（脱敏）",
  detailOccurred: "发生时间",
  detailReceived: "接收时间",
  detailModel: "模型",
  detailMode: "权限模式",
  detailUnmatchedFingerprint: "未匹配项目指纹（仅哈希）",
  detailAttempts: "投递尝试",
  detailNoDocument: "（无正文内容）",
  historyUpdated: "通知历史已更新。",
  unmatchedProject: "未匹配项目",
  // Settings page (Task 19)
  sectionStartup: "启动与窗口",
  autostartLabel: "开机启动",
  closeToTrayLabel: "关闭时最小化到托盘",
  languageLabel: "语言",
  langZh: "中文（简体）",
  langEn: "English",
  localeRestartHint: "语言将在重启应用后生效。",
  themeLabel: "主题",
  themeSystem: "跟随系统",
  themeLight: "浅色",
  themeDark: "深色",
  retentionSection: "数据保留",
  eventRetentionLabel: "历史保留天数",
  logRetentionLabel: "日志保留天数",
  retentionBounds: "保留天数必须是 1–365 之间的整数。",
  savedOk: "设置已保存。",
  settingsLoadFailed: "设置加载失败，请重新打开此页。",
  pauseSection: "通知暂停",
  pausedUntilPrefix: "暂停至：",
  notPaused: "通知未暂停。",
  pause15m: "暂停 15 分钟",
  pause1h: "暂停 1 小时",
  pauseToday: "暂停至今日",
  resumeNotifications: "恢复通知",
  updatesSection: "更新",
  checkUpdates: "检查更新",
  checkingUpdates: "检查中…",
  upToDate: "已是最新版本。",
  updateAvailablePrefix: "发现新版本：",
  installUpdateAction: "安装更新",
  installConfirmTitle: "确认安装更新",
  confirmInstall: "确认安装",
  updateInstalled: "更新程序已启动。",
  credentialStoreSection: "凭据存储",
  debugSection: "调试日志",
  debugLoggingLabel: "调试日志时长",
  debugOff: "关闭",
  debug15m: "15 分钟",
  debug60m: "60 分钟",
  diagnosticsSavedPrefix: "诊断包已保存：",
  diagnosticsCancelled: "已取消导出。",
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
  exportDiagnostics: "Export diagnostics",
  clearHistory: "Clear history",
  clearHistoryWarning: "Completed notification history will be deleted; active jobs and their events are preserved.",
  confirmClearHistory: "Confirm clear history",
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
  colHook: "Hook",
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
  versionConsentDisclosure:
    "The detected agent version is not exactly verified; confirming continues the installation.",
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
  // Preview reflects the SAVED rule; unsaved edits are deliberately excluded.
  previewTitle: "Saved-config preview (redacted)",
  sendTestAction: "Send test",
  sendConfirmTitle: "Confirm test send to",
  confirmSend: "Send now",
  sentOk: "Test sent.",
  // Agent Integration page (Task 18)
  agentDetect: "Detect agents",
  agentDetecting: "Detecting…",
  agentInstallPrefix: "Install ",
  agentRepairPrefix: "Repair ",
  agentUninstallPrefix: "Uninstall ",
  hookWord: " hook",
  colVersion: "Version",
  colState: "State",
  dsDetected: "Detected",
  dsMissing: "Not found",
  dsInvalidVersion: "Invalid version",
  dsProcessFailed: "Detection process failed",
  dsTimedOut: "Detection timed out",
  agentUpgradeNeeded: "Upgrade CC Reminder required",
  upgradeHelperAction: "Upgrade helper",
  uninstallConfirmTitle: "Confirm hook removal",
  uninstallScopeNote:
    "Only hooks created by CC Reminder are removed; the agent's own hooks stay untouched.",
  confirmUninstall: "Uninstall",
  versionConsentTitle: "Continue on an unverified version",
  consentContinue: "Continue",
  lastApplied: "Last applied result",
  ehHealthy: "Healthy",
  ehMissing: "Missing",
  ehDrifted: "Drifted",
  ehHelperMismatch: "Helper mismatch",
  ehNeedsTrust: "Needs confirmation",
  ehAgentUpgradeRequired: "Agent upgrade required",
  trustNotice:
    "Codex hooks need confirmation in the official UI: run the command below, then re-check.",
  // Channels page (Task 18)
  addChannelAction: "Add channel",
  replaceCredentialAction: "Replace credential",
  deleteChannelAction: "Delete channel",
  deleteChannelConfirmTitle: "Delete this channel?",
  deleteChannelNote: "Rules targeting this channel will stop delivering after deletion.",
  confirmDelete: "Delete",
  webhookField: "Webhook",
  signingSecret: "Signing secret (optional)",
  keywordPrefixField: "Keyword prefix",
  savedCredentialBadge: "Credential saved",
  credentialReplaceHint:
    "Saved credentials are never shown; enter a new Webhook to replace them.",
  testSendBtn: "Send test message",
  testSendConfirmTitle: "Confirm test send",
  testSendWarning: "A real test message will be sent to the target group.",
  lastSuccessCol: "Last success",
  neverSucceeded: "Never succeeded",
  pausedBadge: "Paused",
  authPausedNote: "Auth paused: replace the credential.",
  platformCodeLabel: "Platform code",
  markdownFallbackNote:
    "The message was re-sent as plain text because the platform lacks Markdown support.",
  testResultsTitle: "Recent test-send results",
  emptyChannels: "No channels yet",
  channelColName: "Name",
  channelColKind: "Kind",
  channelColCredential: "Credential",
  channelColHealth: "State",
  // Projects page (Task 18)
  addProjectBtn: "Add project",
  projectNameField: "Project name",
  worktreeChoiceLegend: "How should this directory participate in matching?",
  aliasChoiceLabel: "As a path alias of the existing project",
  separateChoiceLabel: "Add as an independent project",
  saveBtn: "Save",
  pickedPathLabel: "Selected folder",
  colProject: "Project",
  colRoot: "Root",
  colAliases: "Path aliases",
  colOverrides: "Overrides",
  removeAliasBtn: "Remove alias",
  removeAliasConfirmTitle: "Remove this path alias?",
  removeAliasNote: "Events under this path will no longer match the project.",
  confirmRemove: "Remove",
  selectAgent: "Select agent",
  allAgentsOption: "All agents",
  scanBoundaryNote:
    "Only the chosen directory and its parents are inspected to find a Git root; the whole disk is never scanned.",
  noProjects: "No projects yet",
  listLoadFailed: "This list failed to load; page data may be incomplete.",
  listSeparator: ", ",
  // Overview page (Task 19)
  metricPending: "{n} pending jobs",
  metricRetry: "{n} jobs waiting to retry",
  metricFailed: "{n} failed jobs",
  metricExpired: "{n} expired jobs",
  metricSpool: "{n} spooled events",
  metricRejected: "{n} rejected events",
  lastSuccessLabel: "Last success: {time}",
  viewFailedJobs: "View failed jobs",
  overviewIssues: "Open issues",
  noIssues: "No open issues.",
  recentFailuresTitle: "Recent failures",
  noRecentFailures: "No recent failures.",
  recentFailuresLoadFailed: "Failed to load recent failures.",
  gotoAgents: "Go to Agent integrations",
  gotoChannels: "Go to channels",
  gotoHistory: "View notification history",
  overviewLoadFailed: "Failed to load health status. Please retry later.",
  refreshedNotice: "Data refreshed.",
  // History page (Task 19)
  filterResult: "Result",
  filterTimeFrom: "From",
  filterTimeUntil: "Until",
  stNotQueued: "Not queued",
  stPending: "Pending",
  stSending: "Sending",
  stRetryWait: "Waiting to retry",
  stSucceeded: "Succeeded",
  stFailed: "Failed",
  stExpired: "Expired",
  colTime: "Time",
  retryFailedJob: "Retry failed job",
  retryExpiredJob: "Retry expired job",
  retryExpiredHint: "Expired jobs cannot be retried.",
  retryConfirmTitle: "Confirm manual retry",
  retryConfirmNote: "The job will be redelivered to its original channel immediately.",
  confirmRetry: "Retry now",
  retryQueued: "Re-delivery queued.",
  loadMore: "Load more",
  emptyHistory: "No notification history",
  detailTitle: "Notification details (redacted)",
  detailOccurred: "Occurred at",
  detailReceived: "Received at",
  detailModel: "Model",
  detailMode: "Permission mode",
  detailUnmatchedFingerprint: "Unmatched project fingerprint (hash only)",
  detailAttempts: "Delivery attempts",
  detailNoDocument: "(no body content)",
  historyUpdated: "Notification history updated.",
  unmatchedProject: "Unmatched project",
  // Settings page (Task 19)
  sectionStartup: "Startup and window",
  autostartLabel: "Launch at login",
  closeToTrayLabel: "Close to tray",
  languageLabel: "Language",
  langZh: "中文（简体）",
  langEn: "English",
  localeRestartHint: "Language changes take effect after restarting the app.",
  themeLabel: "Theme",
  themeSystem: "Follow system",
  themeLight: "Light",
  themeDark: "Dark",
  retentionSection: "Data retention",
  eventRetentionLabel: "History retention (days)",
  logRetentionLabel: "Log retention (days)",
  retentionBounds: "Retention must be an integer between 1 and 365.",
  savedOk: "Settings saved.",
  settingsLoadFailed: "Failed to load settings. Please reopen this page.",
  pauseSection: "Notification pause",
  pausedUntilPrefix: "Paused until:",
  notPaused: "Notifications active.",
  pause15m: "Pause 15 minutes",
  pause1h: "Pause 1 hour",
  pauseToday: "Pause until today ends",
  resumeNotifications: "Resume notifications",
  updatesSection: "Updates",
  checkUpdates: "Check for updates",
  checkingUpdates: "Checking…",
  upToDate: "Up to date.",
  updateAvailablePrefix: "Update available:",
  installUpdateAction: "Install update",
  installConfirmTitle: "Confirm update installation",
  confirmInstall: "Install now",
  updateInstalled: "Updater started.",
  credentialStoreSection: "Credential store",
  debugSection: "Debug logging",
  debugLoggingLabel: "Debug logging duration",
  debugOff: "Off",
  debug15m: "15 minutes",
  debug60m: "60 minutes",
  diagnosticsSavedPrefix: "Diagnostics saved: ",
  diagnosticsCancelled: "Export cancelled.",
};

export function dictionary(locale: LocaleCode): Dictionary {
  return locale === "en" ? en : zhCn;
}

/** Localized DeliveryStatusCode label, shared by overview + history pages. */
export function deliveryStatusText(d: Dictionary, status: string): string {
  switch (status) {
    case "not_queued":
      return d.stNotQueued;
    case "pending":
      return d.stPending;
    case "sending":
      return d.stSending;
    case "retry_wait":
      return d.stRetryWait;
    case "succeeded":
      return d.stSucceeded;
    case "failed":
      return d.stFailed;
    case "expired":
      return d.stExpired;
    default:
      return status;
  }
}
