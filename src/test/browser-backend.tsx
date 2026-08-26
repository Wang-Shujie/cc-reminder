// Deterministic in-browser Backend fake for Playwright acceptance runs
// (Task 21). This module is loaded ONLY when
// VITE_CC_REMINDER_TEST_BACKEND === "1" (see src/App.tsx): Vite statically
// replaces that env var at build time, so production builds eliminate the
// branch and drop this chunk entirely — verified in CI by grepping dist for
// the "cc-reminder-e2e" marker string below.
//
// Determinism knobs, read ONCE from localStorage during construction:
// - cc-reminder-e2e:onboarding = "fresh"  → onboarding_completed: false
// - cc-reminder-e2e:locale     = "en"     → locale "en" (default zh_cn)
//
// Privacy invariant: the fake holds ONE raw credential secret in memory only.
// Every DTO leaving this class passes through assertNoSecret(), so a
// regression throws here instead of leaking into rendered output; the e2e
// suite additionally asserts the marker never appears in the page body.
import type { Backend } from "../lib/backend";
import type {
  AddProjectAliasInput,
  AgentIntegrationSummary,
  ApplyHookActionInput,
  BootstrapState,
  ChannelCredentialInput,
  ChannelId,
  ChannelSummary,
  CoreEventName,
  DeleteChannelInput,
  DiagnosticExportResult,
  GetHistoryDetailInput,
  HealthSnapshot,
  HistoryItem,
  HistoryPage,
  HookInstallationResult,
  HookRuleRow,
  InstallUpdateInput,
  ListHookRulesInput,
  ListHistoryInput,
  LocaleCode,
  ManualRetryInput,
  NotificationDocument,
  PatchFieldCode,
  PreviewNotificationInput,
  ProjectId,
  ProjectSummary,
  RemoveProjectAliasInput,
  ReplaceChannelCredentialInput,
  ResetProjectRuleFieldInput,
  RuleConfig,
  SaveChannelInput,
  SaveGlobalRuleInput,
  SaveProjectInput,
  SaveProjectRulePatchInput,
  SaveSettingsInput,
  SendRuleTestInput,
  SetDebugLoggingInput,
  SetPauseInput,
  SettingsView,
  TestChannelInput,
} from "../lib/contracts";

/** Marker that must NEVER appear in any rendered DTO. */
const E2E_SECRET = "secret-raw-value";

function assertNoSecret(value: unknown): void {
  if (typeof value === "string") {
    if (value.includes(E2E_SECRET)) {
      throw new Error("cc-reminder-e2e: raw secret reached a rendered DTO");
    }
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) assertNoSecret(item);
    return;
  }
  if (value !== null && typeof value === "object") {
    for (const item of Object.values(value)) assertNoSecret(item);
  }
}

interface StoredCredential {
  kind: "ding_talk" | "we_com";
  webhook: string;
  signingSecret: string | null;
}

function defaultRuleConfig(enabled: boolean): RuleConfig {
  return {
    enabled,
    targets: [],
    filters: {
      tool_names: [],
      event_subtypes: [],
      permission_modes: [],
      models: [],
      statuses: [],
    },
    privacy: {
      allowed_sensitive_fields: [],
      max_body_chars: 0,
      summary_mode: "metadata_only",
      extra_redaction_patterns: [],
    },
    delivery: {
      mode: { mode: "immediate" },
      cooldown_seconds: 0,
      max_per_window: 100,
      window_seconds: 3_600,
      quiet_behavior: "suppress",
      ttl_seconds: 1_800,
      max_attempts: 5,
    },
    quiet_hours: null,
  };
}

interface RuleSeed {
  event: string;
  enabled?: boolean;
  phase: string;
  available?: boolean;
  highFrequency?: boolean;
  status?: "stable" | "experimental" | "deprecated";
}

function mkRule(agent: "claude-code" | "codex", seed: RuleSeed): HookRuleRow {
  const available = seed.available ?? true;
  const config = defaultRuleConfig(seed.enabled ?? false);
  return {
    agent,
    source_event: seed.event,
    enabled: config.enabled,
    version: 1,
    config,
    patched_fields: [],
    installed: Boolean(config.enabled) && available,
    phase: seed.phase,
    sensitivity: "sensitive",
    high_frequency: seed.highFrequency ?? false,
    status: seed.status ?? "stable",
    available,
    input_fields: [
      { name: "session_id", sensitivity: "sensitive" },
      { name: "tool_input", sensitivity: "sensitive" },
      { name: "tool_name", sensitivity: "public" },
    ],
  };
}

function claudeRules(): HookRuleRow[] {
  return [
    mkRule("claude-code", { event: "PermissionRequest", enabled: true, phase: "request" }),
    mkRule("claude-code", {
      event: "PostToolUseFailure",
      phase: "failure",
      available: false,
    }),
    mkRule("claude-code", { event: "PreToolUse", phase: "before", highFrequency: true }),
    mkRule("claude-code", { event: "Stop", enabled: true, phase: "stop" }),
    mkRule("claude-code", { event: "Elicitation", phase: "request", status: "experimental" }),
    mkRule("claude-code", { event: "TaskCreated", phase: "create", status: "deprecated" }),
  ];
}

function codexRules(): HookRuleRow[] {
  return [
    mkRule("codex", { event: "PermissionRequest", enabled: true, phase: "request" }),
    mkRule("codex", { event: "SessionEnd", phase: "end" }),
    mkRule("codex", { event: "Stop", enabled: true, phase: "stop" }),
  ];
}

function detectedAgents(): AgentIntegrationSummary[] {
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
      needs_compatible_version_confirmation: false,
    },
  ];
}

const PROJECT_ID = "3fa85f64-5717-4562-b3fc-2c963f66afa6" as ProjectId;

function seedProjects(): ProjectSummary[] {
  return [
    {
      id: PROJECT_ID,
      name: "演示项目",
      canonical_root: "/home/demo/client-app",
      worktree_mode: "alias",
      paths: [{ id: "path-1", kind: "alias", canonical_path: "/home/demo/client-app-worktrees" }],
      override_count: 0,
    },
  ];
}

function seedHistory(channels: StoredChannelSummary[]): HistoryItem[] {
  const primaryChannel = channels[0]?.id ?? null;
  const base = {
    source: "claude-code",
    source_version: "2.1.218",
    category: "lifecycle",
    correlation_id: "corr-e2e-000",
    processing_outcome: "queued",
    outcome_reason_code: null,
    project_id: PROJECT_ID,
    project_display_name: "演示项目",
    unmatched_cwd_fingerprint: null,
    model: "claude-opus-4-6",
    permission_mode: "default",
    severity: "info",
    public_fields: {},
    channel_id: primaryChannel,
  } as const;
  return [
    {
      ...base,
      event_id: "evt-e2e-0001",
      source_event: "Stop",
      occurred_at: "2026-08-20T02:30:00Z",
      received_at: "2026-08-20T02:30:01Z",
      unmatched_cwd_fingerprint: null,
      public_fields: { tool_name: "Bash" },
      delivery_job_id: "job-e2e-0001",
      channel_id: primaryChannel,
      document: {
        title: "Stop：任务完成",
        severity: "info",
        facts: [
          ["事件", "Stop"],
          ["项目", "演示项目"],
        ],
        body: "",
        footer: null,
      },
      delivery_status: "succeeded",
      attempts: [
        {
          attempt_number: 1,
          started_at: "2026-08-20T02:30:01Z",
          completed_at: "2026-08-20T02:30:02Z",
          outcome: "succeeded",
          http_status: 200,
          platform_code: null,
          error_code: null,
          retry_at: null,
          redacted_detail: null,
        },
      ],
    },
    {
      ...base,
      event_id: "evt-e2e-0002",
      source_event: "PermissionRequest",
      category: "permission",
      severity: "warning",
      occurred_at: "2026-08-20T03:10:00Z",
      received_at: "2026-08-20T03:10:01Z",
      delivery_job_id: "job-e2e-0002",
      document: {
        title: "PermissionRequest：等待确认",
        severity: "warning",
        facts: [
          ["事件", "PermissionRequest"],
          ["工具", "Bash"],
        ],
        body: "",
        footer: null,
      },
      delivery_status: "failed",
      attempts: [
        {
          attempt_number: 1,
          started_at: "2026-08-20T03:10:01Z",
          completed_at: "2026-08-20T03:10:05Z",
          outcome: "failed",
          http_status: 500,
          platform_code: null,
          error_code: "delivery.http_error",
          retry_at: "2026-08-20T03:11:00Z",
          redacted_detail: "远端返回 500",
        },
        {
          attempt_number: 2,
          started_at: "2026-08-20T03:11:00Z",
          completed_at: "2026-08-20T03:11:04Z",
          outcome: "failed",
          http_status: 500,
          platform_code: null,
          error_code: "delivery.http_error",
          retry_at: "2026-08-20T03:13:00Z",
          redacted_detail: "远端返回 500",
        },
        {
          attempt_number: 3,
          started_at: "2026-08-20T03:13:00Z",
          completed_at: "2026-08-20T03:13:04Z",
          outcome: "failed",
          http_status: 502,
          platform_code: null,
          error_code: "delivery.http_error",
          retry_at: null,
          redacted_detail: "远端返回 502，已达最大尝试次数",
        },
      ],
    },
    {
      ...base,
      event_id: "evt-e2e-0003",
      source_event: "PreToolUse",
      category: "tool",
      occurred_at: "2026-08-20T04:00:00Z",
      received_at: "2026-08-20T04:00:01Z",
      project_display_name: null,
      unmatched_cwd_fingerprint: "9f2c41d7a80b3e65c1d204f7b8a93e10",
      permission_mode: "bypassPermissions",
      delivery_job_id: null,
      document: null,
      delivery_status: "not_queued",
      attempts: [],
    },
  ];
}

type StoredChannelSummary = Omit<ChannelSummary, "id"> & { id: ChannelId };

export class BrowserTestBackend implements Backend {
  private readonly handlers = new Map<CoreEventName, Set<(revision: number) => void>>();
  private channelsState: StoredChannelSummary[];
  private credentials: Map<ChannelId, StoredCredential>;
  private rulesState: HookRuleRow[];
  private patchesState: Map<string, Partial<RuleConfig>>;
  private projectsState: ProjectSummary[];
  private historyState: HistoryItem[];
  private settings: SettingsView;
  private nextChannelId = 3;
  private nextPathId = 2;

  constructor() {
    // Determinism knobs are read once; nothing else consults the environment.
    const fresh = window.localStorage.getItem("cc-reminder-e2e:onboarding") === "fresh";
    const locale =
      window.localStorage.getItem("cc-reminder-e2e:locale") === "en" ? "en" : "zh_cn";

    this.credentials = new Map();
    const wecomId = "ch-e2e-1" as ChannelId;
    const dingId = "ch-e2e-2" as ChannelId;
    this.credentials.set(wecomId, {
      kind: "we_com",
      webhook: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=demo-wecom",
      signingSecret: null,
    });
    this.credentials.set(dingId, {
      kind: "ding_talk",
      webhook: "https://oapi.dingtalk.com/robot/send?access_token=demo-dingtalk",
      // Raw material lives HERE only; it must never surface in a read model.
      signingSecret: E2E_SECRET,
    });
    const configuredChannels: StoredChannelSummary[] = [
      {
        id: wecomId,
        kind: "we_com",
        name: "值班群",
        credential_present: true,
        health: "healthy",
        paused: false,
        last_succeeded_at: "2026-08-20T02:30:02Z",
      },
      {
        id: dingId,
        kind: "ding_talk",
        name: "钉钉值班群",
        credential_present: true,
        health: "unknown",
        paused: false,
        last_succeeded_at: null,
      },
    ];
    // A genuinely fresh install has no channels, history or projects yet, so
    // onboarding resumes at its FIRST step instead of jumping to defaults.
    this.channelsState = fresh ? [] : configuredChannels;
    this.rulesState = [...claudeRules(), ...codexRules()];
    this.patchesState = new Map();
    this.projectsState = fresh ? [] : seedProjects();
    this.historyState = fresh ? [] : seedHistory(this.channelsState);
    this.settings = {
      autostart: false,
      close_to_tray: true,
      locale,
      theme: "system",
      event_retention_days: 30,
      log_retention_days: 7,
      onboarding_completed: !fresh,
      paused_until: null,
    };
  }

  /** Guard every outbound DTO. */
  private send<T>(value: T): T {
    assertNoSecret(value);
    return value;
  }

  private snapshot(): HealthSnapshot {
    return {
      overall: "ok",
      agents: [],
      channels: [],
      pending_jobs: 1,
      retry_jobs: 0,
      failed_jobs: 1,
      expired_jobs: 0,
      spool_count: 0,
      rejected_count: 0,
      last_success_at: "2026-08-20T02:30:02Z",
      issues: [],
    };
  }

  private mergedRows(input: ListHookRulesInput): HookRuleRow[] {
    const rows = structuredClone(this.rulesState).filter((row) => row.agent === input.agent);
    if (!input.project_id) {
      return rows;
    }
    return rows.map((row) => {
      const key = `${input.project_id as string}:${row.agent}:${row.source_event}`;
      const patch = this.patchesState.get(key);
      if (!patch) {
        return row;
      }
      const defined = Object.entries(patch).filter(([, value]) => value !== undefined);
      const patchedFields = defined.map(([field]) => field) as PatchFieldCode[];
      const config = { ...row.config };
      for (const [field, value] of defined) {
        (config as Record<string, unknown>)[field] = structuredClone(value);
      }
      return { ...row, enabled: config.enabled, config, patched_fields: patchedFields };
    });
  }

  // -- Bootstrap + health ---------------------------------------------------

  async getBootstrapState(_offsetSeconds?: number | null): Promise<BootstrapState> {
    // The in-browser backend keeps its fixed settings; the reported offset is
    // accepted for signature parity with the real core but not modelled here.
    const snap = this.snapshot();
    return this.send({
      onboarding_completed: this.settings.onboarding_completed,
      locale: this.settings.locale,
      theme: this.settings.theme,
      health: snap,
      pending_jobs: snap.pending_jobs,
      failed_jobs: snap.failed_jobs,
    });
  }

  async getHealthSnapshot(): Promise<HealthSnapshot> {
    return this.send(this.snapshot());
  }

  async detectAgents(): Promise<AgentIntegrationSummary[]> {
    return this.send(detectedAgents());
  }

  async listAgentIntegrations(): Promise<AgentIntegrationSummary[]> {
    return this.send(detectedAgents());
  }

  async applyHookAction(input: ApplyHookActionInput): Promise<HookInstallationResult> {
    this.rulesState = this.rulesState.map((row) =>
      row.agent !== input.agent ? row : { ...row, installed: row.enabled && row.available },
    );
    return this.send({
      agent: input.agent,
      selection_out_of_date: false,
      entries: [],
    });
  }

  // -- Rules ----------------------------------------------------------------

  async listHookRules(input: ListHookRulesInput): Promise<HookRuleRow[]> {
    return this.send(this.mergedRows(input));
  }

  async saveGlobalRule(input: SaveGlobalRuleInput): Promise<HookRuleRow> {
    const index = this.rulesState.findIndex(
      (r) => r.agent === input.agent && r.source_event === input.source_event,
    );
    const row: HookRuleRow = {
      agent: input.agent,
      source_event: input.source_event,
      enabled: input.config.enabled,
      version: 2,
      config: structuredClone(input.config),
      patched_fields: [],
      installed: input.config.enabled,
      phase: index >= 0 ? (this.rulesState[index]?.phase ?? "stop") : "stop",
      sensitivity: "sensitive",
      high_frequency: false,
      status: "stable",
      available: true,
      input_fields:
        index >= 0 ? (this.rulesState[index]?.input_fields ?? []) : [],
    };
    if (index >= 0) {
      this.rulesState[index] = row;
    } else {
      this.rulesState.push(row);
    }
    return this.send(structuredClone(row));
  }

  async saveProjectRulePatch(input: SaveProjectRulePatchInput): Promise<void> {
    const key = `${input.project_id}:${input.agent}:${input.source_event}`;
    const merged: Partial<RuleConfig> = { ...(this.patchesState.get(key) ?? {}) };
    for (const [field, value] of Object.entries(input.patch)) {
      if (value === undefined) continue;
      (merged as Record<string, unknown>)[field] = structuredClone(value);
    }
    this.patchesState.set(key, merged);
  }

  async resetProjectRuleField(input: ResetProjectRuleFieldInput): Promise<void> {
    const key = `${input.project_id}:${input.agent}:${input.source_event}`;
    const existing = this.patchesState.get(key);
    if (!existing) return;
    const rest = { ...existing };
    delete rest[input.field];
    if (Object.keys(rest).length === 0) {
      this.patchesState.delete(key);
    } else {
      this.patchesState.set(key, rest);
    }
  }

  async previewNotification(input: PreviewNotificationInput): Promise<NotificationDocument> {
    return this.send({
      title: `预览：${input.source_event}`,
      severity: "info",
      facts: [["事件", input.source_event]],
      // Metadata-only by default: no body content is ever previewed here.
      body: "",
      footer: null,
    });
  }

  async sendRuleTest(_input: SendRuleTestInput): Promise<void> {
    void _input;
  }

  // -- Channels ---------------------------------------------------------------

  private channelSummaries(): ChannelSummary[] {
    return this.channelsState.map(({ id, kind, name, credential_present, health, paused, last_succeeded_at }) => ({
      id,
      kind,
      name,
      credential_present,
      health,
      paused,
      last_succeeded_at,
    }));
  }

  async listChannels(): Promise<ChannelSummary[]> {
    return this.send(this.channelSummaries());
  }

  private static assertOfficialWebhook(credential: ChannelCredentialInput): void {
    const ok =
      credential.kind === "ding_talk"
        ? credential.webhook.startsWith("https://oapi.dingtalk.com/")
        : credential.webhook.startsWith("https://qyapi.weixin.qq.com/");
    if (!ok) {
      throw {
        code: "configuration.invalid_webhook",
        message: "webhook rejected: unofficial host",
      };
    }
  }

  async saveChannel(input: SaveChannelInput): Promise<ChannelSummary> {
    BrowserTestBackend.assertOfficialWebhook(input.credential);
    const id = `ch-e2e-${this.nextChannelId++}` as ChannelId;
    this.credentials.set(id, {
      kind: input.credential.kind,
      webhook: input.credential.webhook,
      signingSecret:
        input.credential.kind === "ding_talk"
          ? (input.credential.signing_secret ?? null)
          : null,
    });
    const saved: StoredChannelSummary = {
      id,
      kind: input.credential.kind,
      name: input.name,
      credential_present: true,
      health: "unknown",
      paused: false,
      last_succeeded_at: null,
    };
    this.channelsState.push(saved);
    return this.send({ ...saved });
  }

  async replaceChannelCredential(input: ReplaceChannelCredentialInput): Promise<ChannelSummary> {
    const existing = this.channelsState.find((c) => c.id === input.channel_id);
    if (!existing) {
      throw { code: "history.event_not_found", message: "unknown channel" };
    }
    BrowserTestBackend.assertOfficialWebhook(input.credential);
    if (input.credential.kind !== existing.kind) {
      throw {
        code: "configuration.channel_kind_mismatch",
        message: "credential kind does not match channel",
      };
    }
    this.credentials.set(input.channel_id, {
      kind: input.credential.kind,
      webhook: input.credential.webhook,
      signingSecret:
        input.credential.kind === "ding_talk"
          ? (input.credential.signing_secret ?? null)
          : null,
    });
    existing.health = "unknown";
    return this.send({ ...existing });
  }

  async deleteChannel(input: DeleteChannelInput): Promise<void> {
    this.channelsState = this.channelsState.filter((c) => c.id !== input.channel_id);
    this.credentials.delete(input.channel_id);
  }

  async testChannel(input: TestChannelInput): Promise<{ http_status: number; platform_code: string | null }> {
    void input.channel_id;
    return { http_status: 200, platform_code: null };
  }

  // -- Projects ---------------------------------------------------------------

  async listProjects(): Promise<ProjectSummary[]> {
    return this.send(structuredClone(this.projectsState));
  }

  async saveProject(input: SaveProjectInput): Promise<ProjectSummary> {
    const saved: ProjectSummary = {
      id: input.project_id ?? (`proj-e2e-${this.projectsState.length + 1}` as ProjectId),
      name: input.name,
      canonical_root: input.canonical_root,
      worktree_mode: input.worktree_mode,
      paths: [],
      override_count: 0,
    };
    this.projectsState.push(saved);
    return this.send(structuredClone(saved));
  }

  async addProjectAlias(input: AddProjectAliasInput): Promise<void> {
    const owner = this.projectsState.find((p) => p.id === input.project_id);
    owner?.paths.push({
      id: `path-${this.nextPathId++}`,
      kind: "alias",
      canonical_path: input.canonical_path,
    });
  }

  async removeProjectAlias(input: RemoveProjectAliasInput): Promise<void> {
    for (const project of this.projectsState) {
      project.paths = project.paths.filter((path) => path.id !== input.path_id);
    }
  }

  // -- History ------------------------------------------------------------------

  async listHistory(input: ListHistoryInput = {}): Promise<HistoryPage> {
    const limit = input.limit ?? 50;
    const offset = input.offset ?? 0;
    const filtered = this.historyState.filter((item) => {
      if (input.delivery_status && item.delivery_status !== input.delivery_status) return false;
      if (input.project_id && item.project_id !== input.project_id) return false;
      if (input.source_event && item.source_event !== input.source_event) return false;
      if (input.channel_id && item.channel_id !== input.channel_id) return false;
      if (input.occurred_from && item.occurred_at < input.occurred_from) return false;
      if (input.occurred_until && item.occurred_at > input.occurred_until) return false;
      return true;
    });
    const slice = filtered.slice(offset, offset + limit);
    const next = offset + slice.length < filtered.length ? offset + slice.length : null;
    return this.send({ items: structuredClone(slice), next_offset: next });
  }

  async getHistoryDetail(input: GetHistoryDetailInput): Promise<HistoryPage> {
    const found = this.historyState.find((item) => item.event_id === input.event_id);
    if (!found) {
      throw { code: "history.event_not_found", message: "event not found" };
    }
    return this.send({ items: [structuredClone(found)], next_offset: null });
  }

  async manualRetryDelivery(input: ManualRetryInput): Promise<void> {
    const item = this.historyState.find((h) => h.delivery_job_id === input.job_id);
    if (item) {
      item.delivery_status = "pending";
      item.attempts = [];
    }
  }

  // -- Settings + maintenance ------------------------------------------------------

  async getSettings(): Promise<SettingsView> {
    return this.send(structuredClone(this.settings));
  }

  async saveSettings(input: SaveSettingsInput): Promise<SettingsView> {
    this.settings = { ...this.settings, ...input };
    return this.send(structuredClone(this.settings));
  }

  async setNotificationPause(input: SetPauseInput): Promise<SettingsView> {
    const now = new Date();
    let until: Date;
    if (input.duration === "fifteen_minutes") {
      until = new Date(now.getTime() + 15 * 60_000);
    } else if (input.duration === "one_hour") {
      until = new Date(now.getTime() + 60 * 60_000);
    } else {
      until = new Date(now);
      until.setHours(24, 0, 0, 0);
    }
    this.settings.paused_until = until.toISOString();
    return this.send(structuredClone(this.settings));
  }

  async clearNotificationPause(): Promise<SettingsView> {
    this.settings.paused_until = null;
    return this.send(structuredClone(this.settings));
  }

  async checkForUpdates(): Promise<{ available: false; version: null; notes: null; installable: false }> {
    return { available: false, version: null, notes: null, installable: false };
  }

  async installUpdate(_input: InstallUpdateInput): Promise<void> {
    void _input;
  }

  async exportDiagnostics(): Promise<DiagnosticExportResult> {
    return { status: "saved", filename: "cc-reminder-diagnostics.zip" };
  }

  async clearHistory(input: { preserve_active_jobs: boolean }): Promise<number> {
    const before = this.historyState.length;
    this.historyState =
      input.preserve_active_jobs
        ? this.historyState.filter(
            (item) => item.delivery_status === "pending" || item.delivery_status === "sending",
          )
        : [];
    return before - this.historyState.length;
  }

  async setDebugLogging(input: SetDebugLoggingInput): Promise<SettingsView> {
    void input;
    return this.send(structuredClone(this.settings));
  }

  // -- Events -------------------------------------------------------------

  async subscribe(
    event: CoreEventName,
    handler: (revision: number) => void,
  ): Promise<() => void> {
    let set = this.handlers.get(event);
    if (!set) {
      set = new Set();
      this.handlers.set(event, set);
    }
    set.add(handler);
    return () => {
      set.delete(handler);
    };
  }
}

/** Entry point used by src/main.tsx behind the compile-time test flag. */
export function createBrowserTestBackend(): Backend {
  window.localStorage.setItem("cc-reminder-e2e", "browser-test-backend");
  return new BrowserTestBackend();
}
