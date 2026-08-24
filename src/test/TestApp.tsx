// Deterministic in-memory Backend fake + render harness shared by the
// frontend tests. Production code never imports this module.
import { BackendProvider } from "../lib/backend";
import { AppRoot } from "../App";
import type { Backend } from "../lib/backend";
import type {
  AddProjectAliasInput,
  AgentIntegrationSummary,
  AgentKindCode,
  BootstrapState,
  CapabilityStatusCode,
  ChannelCredentialInput,
  ChannelId,
  ChannelSummary,
  CoreEventName,
  DeleteChannelInput,
  GetHistoryDetailInput,
  HealthSnapshot,
  HistoryItem,
  HistoryPage,
  HookApplyEntry,
  HookInstallationResult,
  HookRuleRow,
  InstallUpdateInput,
  ListHookRulesInput,
  ListHistoryInput,
  LocaleCode,
  ManualRetryInput,
  PatchFieldCode,
  ProjectId,
  ProjectSummary,
  RemoveProjectAliasInput,
  ReplaceChannelCredentialInput,
  RuleConfig,
  SaveChannelInput,
  SaveProjectInput,
  SaveSettingsInput,
  SensitivityCode,
  SettingsView,
  SetPauseInput,
  TestChannelInput,
  ThemeCode,
  UpdateCheckResult,
} from "../lib/contracts";
import type { ReactNode } from "react";

/** Serialized AppError shape the real Tauri bridge rejects with. */
export interface FakeAppError {
  code: string;
  message: string;
  suggested_action?: string | null;
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

/** Mirrors channels::validate_official_webhook: only the platforms' official
 *  webhook hosts are accepted, before any credential would be stored. */
function assertOfficialWebhook(credential: ChannelCredentialInput): void {
  const host = credential.webhook;
  const ok =
    credential.kind === "ding_talk"
      ? host.startsWith("https://oapi.dingtalk.com/")
      : host.startsWith("https://qyapi.weixin.qq.com/");
  if (!ok) {
    throw {
      code: "configuration.invalid_webhook",
      message: "webhook rejected: unofficial host",
    };
  }
}

export function okSnapshot(): HealthSnapshot {
  return {
    overall: "ok",
    agents: [],
    channels: [],
    pending_jobs: 1,
    retry_jobs: 0,
    failed_jobs: 0,
    expired_jobs: 0,
    spool_count: 0,
    rejected_count: 0,
    last_success_at: null,
    issues: [],
  };
}

export interface FakeBackendOptions {
  onboardingCompleted?: boolean;
  locale?: LocaleCode;
  theme?: ThemeCode;
  detectResults?: () => AgentIntegrationSummary[];
  channels?: ChannelSummary[];
  rules?: HookRuleRow[];
  /** Keyed `${project_id}:${agent}:${source_event}` → present top-level fields. */
  projectPatches?: Record<string, Partial<RuleConfig>>;
  /** Projects returned by list_projects (needed for cross-scope drift tests). */
  projects?: ProjectSummary[];
  /** First apply_hook_action without consent rejects with
   *  integration.agent_confirmation_required (F2 gate simulation). */
  applyConfirmationRequired?: boolean;
  /** Entries returned by a successful apply_hook_action (per-event health). */
  applyEntries?: () => HookApplyEntry[];
  /** When set, every apply_hook_action rejects with this AppError shape. */
  applyError?: FakeAppError;
  selectionOutOfDate?: boolean;
  previewBody?: string;
  previewError?: string;
  sendError?: string;
  /** Channel ids whose deletion is refused (configuration.channel_in_use). */
  channelInUseIds?: ChannelId[];
  testChannelResult?: { http_status: number; platform_code: string | null };
  testChannelError?: FakeAppError;
  /** When true, project saves/alias adds reject with configuration.path_conflict. */
  projectConflict?: boolean;
  /** Merged over the neutral ok snapshot (Task 19 overview/settings tests). */
  health?: Partial<HealthSnapshot>;
  /** getHealthSnapshot rejects like the serialized AppError when set. */
  snapshotError?: boolean;
  /** In-memory history rows; listHistory filters + paginates them for real. */
  historyItems?: HistoryItem[];
  /** listHistory rejects with this serialized AppError when set. */
  historyListError?: FakeAppError;
  /** manualRetryDelivery rejects with this serialized AppError when set. */
  retryError?: FakeAppError;
  /** Result returned by checkForUpdates. */
  updateCheck?: UpdateCheckResult;
  /** installUpdate rejects with this serialized AppError when set. */
  installUpdateError?: FakeAppError;
}

/** Neutral default RuleConfig matching rules::resolve::default_rule. */
export function defaultRuleConfig(enabled = false): RuleConfig {
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

export interface RuleFixture {
  agent: AgentKindCode;
  source_event: string;
  enabled?: boolean;
  phase?: string;
  sensitivity?: SensitivityCode;
  high_frequency?: boolean;
  status?: CapabilityStatusCode;
  available?: boolean;
  installed?: boolean;
  config?: Partial<RuleConfig>;
}

function mkRow(fixture: RuleFixture): HookRuleRow {
  const config = { ...defaultRuleConfig(fixture.enabled ?? false), ...fixture.config };
  const available = fixture.available ?? true;
  return {
    agent: fixture.agent,
    source_event: fixture.source_event,
    enabled: config.enabled,
    version: 1,
    config,
    patched_fields: [],
    // Steady-state invariant (installed == enabled∧available) by default so
    // drift stays row-derivable; tests opt out explicitly.
    installed: fixture.installed ?? Boolean(config.enabled && available),
    phase: fixture.phase ?? "stop",
    sensitivity: fixture.sensitivity ?? "sensitive",
    high_frequency: fixture.high_frequency ?? false,
    status: fixture.status ?? "stable",
    available,
    input_fields: [
      { name: "session_id", sensitivity: "sensitive" },
      { name: "tool_input", sensitivity: "sensitive" },
      { name: "tool_name", sensitivity: "public" },
    ],
  };
}

export function testChannelSummary(): ChannelSummary {
  return {
    id: "ch-1" as ChannelId,
    kind: "we_com",
    name: "值班群",
    credential_present: true,
    health: "unknown",
    paused: false,
    last_succeeded_at: null,
  };
}

/** Claude Code table fixtures: unavailable, high-frequency, experimental and
 *  deprecated rows plus the two rows the drawer tests open. No other event
 *  name contains "Stop" so row-name regexes stay unambiguous. */
export function claudeRulesFixtures(): HookRuleRow[] {
  return [
    mkRow({
      agent: "claude-code",
      source_event: "PermissionRequest",
      enabled: true,
      // Mirrors the real catalog vocabulary (resources/capabilities/*.json).
      phase: "request",
    }),
    mkRow({
      agent: "claude-code",
      source_event: "PostToolUseFailure",
      phase: "failure",
      available: false,
      installed: false,
    }),
    mkRow({
      agent: "claude-code",
      source_event: "PreToolUse",
      high_frequency: true,
      phase: "before",
    }),
    mkRow({ agent: "claude-code", source_event: "Stop", enabled: true }),
    mkRow({
      agent: "claude-code",
      source_event: "Elicitation",
      status: "experimental",
      phase: "request",
    }),
    mkRow({
      agent: "claude-code",
      source_event: "TaskCreated",
      status: "deprecated",
      phase: "create",
    }),
  ];
}

/** Small Codex fixture set for tab-scoping tests. */
export function codexRulesFixtures(): HookRuleRow[] {
  return [
    mkRow({
      agent: "codex",
      source_event: "PermissionRequest",
      enabled: true,
      phase: "request",
    }),
    mkRow({ agent: "codex", source_event: "SessionEnd", phase: "end", installed: false }),
    mkRow({ agent: "codex", source_event: "Stop", enabled: true }),
  ];
}

export interface RulesScope {
  scope: "global";
}
export interface ProjectRulesScope {
  scope: "project";
  project_id: string;
  project_name: string;
}

export const PROJECT_ID = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

export function globalScope(): RulesScope {
  return { scope: "global" };
}

export function projectScope(): ProjectRulesScope {
  return { scope: "project", project_id: PROJECT_ID, project_name: "演示项目" };
}

const patchKey = (scope: ProjectRulesScope["project_id"], agent: string, event: string): string =>
  `${scope}:${agent}:${event}`;

/** Two-row project fixture used by the inheritance tests. */
export function projectRulesBackendOptions(): FakeBackendOptions {
  return {
    channels: [testChannelSummary()],
    rules: [
      mkRow({
        agent: "claude-code",
        source_event: "PermissionRequest",
        enabled: true,
        phase: "request",
      }),
      mkRow({ agent: "claude-code", source_event: "Stop", enabled: true }),
    ],
  };
}

export class FakeBackend implements Backend {
  readonly handlers = new Map<CoreEventName, Set<(revision: number) => void>>();
  readonly getHealthSnapshot: Backend["getHealthSnapshot"];
  readonly saveSettings: Backend["saveSettings"];
  readonly sendRuleTest: Backend["sendRuleTest"];
  readonly detectAgents: Backend["detectAgents"];
  readonly applyHookAction: Backend["applyHookAction"];
  readonly saveGlobalRule: Backend["saveGlobalRule"];
  readonly saveProjectRulePatch: Backend["saveProjectRulePatch"];
  readonly resetProjectRuleField: Backend["resetProjectRuleField"];
  readonly previewNotification: Backend["previewNotification"];

  private opts: Required<
    Omit<
      FakeBackendOptions,
      | "detectResults"
      | "channels"
      | "rules"
      | "projectPatches"
      | "projects"
      | "applyConfirmationRequired"
      | "applyEntries"
      | "applyError"
      | "selectionOutOfDate"
      | "previewBody"
      | "previewError"
      | "sendError"
      | "channelInUseIds"
      | "testChannelResult"
      | "testChannelError"
      | "projectConflict"
      | "health"
      | "snapshotError"
      | "historyItems"
      | "historyListError"
      | "retryError"
      | "updateCheck"
      | "installUpdateError"
    >
  > & Pick<
    FakeBackendOptions,
    "detectResults" | "previewBody" | "previewError" | "sendError" | "applyEntries"
  >;
  private healthOverride: Partial<HealthSnapshot> | undefined;
  private snapshotError: boolean | undefined;
  private historyItemsState: HistoryItem[];
  private historyListError: FakeAppError | undefined;
  private retryError: FakeAppError | undefined;
  private updateCheckResult: UpdateCheckResult;
  private installUpdateError: FakeAppError | undefined;
  private channelsState: ChannelSummary[];
  private rulesState: HookRuleRow[];
  private patchesState: Map<string, Partial<RuleConfig>>;
  private applyConfirmationRequired: boolean;
  private applyError: FakeAppError | undefined;
  private projectsState: ProjectSummary[];
  private settings: SettingsView;
  private savedSettingsInputs: SaveSettingsInput[] = [];
  private selectionOutOfDate: boolean;
  private nextChannelId = 1;
  private channelInUseIds: ChannelId[];
  private testChannelResult: { http_status: number; platform_code: string | null };
  private testChannelError: FakeAppError | undefined;
  private projectConflict: boolean;
  private nextPathId = 1;

  readonly saveChannel: Backend["saveChannel"];
  readonly replaceChannelCredential: Backend["replaceChannelCredential"];
  readonly deleteChannel: Backend["deleteChannel"];
  readonly testChannel: Backend["testChannel"];
  readonly saveProject: Backend["saveProject"];
  readonly addProjectAlias: Backend["addProjectAlias"];
  readonly removeProjectAlias: Backend["removeProjectAlias"];
  readonly listHistory: Backend["listHistory"];
  readonly getHistoryDetail: Backend["getHistoryDetail"];
  readonly manualRetryDelivery: Backend["manualRetryDelivery"];
  readonly getSettings: Backend["getSettings"];
  readonly setNotificationPause: Backend["setNotificationPause"];
  readonly clearNotificationPause: Backend["clearNotificationPause"];
  readonly checkForUpdates: Backend["checkForUpdates"];
  readonly installUpdate: Backend["installUpdate"];

  constructor(options: FakeBackendOptions = {}) {
    this.opts = {
      onboardingCompleted: options.onboardingCompleted ?? false,
      locale: options.locale ?? "zh_cn",
      theme: options.theme ?? "system",
      detectResults: options.detectResults ?? (() => [
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
      ]),
      previewBody: options.previewBody,
      previewError: options.previewError,
      sendError: options.sendError,
      applyEntries: options.applyEntries,
    };
    this.applyError = options.applyError;
    this.channelInUseIds = options.channelInUseIds ? [...options.channelInUseIds] : [];
    this.testChannelResult =
      options.testChannelResult ?? { http_status: 200, platform_code: null };
    this.testChannelError = options.testChannelError;
    this.projectConflict = options.projectConflict ?? false;
    this.healthOverride = options.health;
    this.snapshotError = options.snapshotError;
    this.historyItemsState = options.historyItems ? clone(options.historyItems) : [];
    this.historyListError = options.historyListError;
    this.retryError = options.retryError;
    this.updateCheckResult =
      options.updateCheck ?? {
        available: false,
        version: null,
        notes: null,
        installable: false,
      };
    this.installUpdateError = options.installUpdateError;
    this.channelsState = options.channels ? clone(options.channels) : [];
    this.rulesState = options.rules ? clone(options.rules) : [];
    this.patchesState = new Map(
      Object.entries(options.projectPatches ?? {}).map(([key, patch]) => [
        key,
        clone(patch),
      ]),
    );
    this.applyConfirmationRequired = options.applyConfirmationRequired ?? false;
    this.projectsState = options.projects ? clone(options.projects) : [];
    this.selectionOutOfDate = options.selectionOutOfDate ?? false;
    this.settings = {
      autostart: false,
      close_to_tray: true,
      locale: this.opts.locale,
      theme: this.opts.theme,
      event_retention_days: 30,
      log_retention_days: 7,
      onboarding_completed: this.opts.onboardingCompleted,
      paused_until: null,
    };
    this.getHealthSnapshot = vi.fn(async (): Promise<HealthSnapshot> => {
      if (this.snapshotError) {
        throw { code: "health.unavailable", message: "健康状态暂不可用。" };
      }
      return clone(this.snapshot());
    });
    this.saveSettings = vi.fn(async (input: SaveSettingsInput): Promise<SettingsView> => {
      this.savedSettingsInputs.push(clone(input));
      this.settings = { ...this.settings, ...input };
      return clone(this.settings);
    });
    this.sendRuleTest = vi.fn(async (input): Promise<void> => {
      if (this.opts.sendError) {
        throw new Error(this.opts.sendError);
      }
      void input;
    });
    this.detectAgents = vi.fn(
      async (_input: { confirm_compatible_version: boolean }): Promise<AgentIntegrationSummary[]> =>
        clone(this.opts.detectResults?.() ?? []),
    );
    this.applyHookAction = vi.fn(
      async (input): Promise<HookInstallationResult> => {
        if (this.applyError) {
          throw { ...this.applyError };
        }
        if (this.applyConfirmationRequired && !input.confirm_compatible_version) {
          // Serialized shape mirrors the Rust integration_error DTO.
          throw {
            code: "integration.agent_confirmation_required",
            message: "agent version requires explicit confirmation",
          };
        }
        this.selectionOutOfDate = false;
        // The repair reconciles installed hooks with the required selection:
        // enabled∧available events become installed, everything else removed.
        this.rulesState = this.rulesState.map((row) =>
          row.agent !== input.agent
            ? row
            : { ...row, installed: row.enabled && row.available },
        );
        return {
          agent: input.agent,
          selection_out_of_date: false,
          entries: this.opts.applyEntries?.() ?? [],
        };
      },
    );
    this.saveGlobalRule = vi.fn(async (input) => {
      const row: HookRuleRow = {
        agent: input.agent,
        source_event: input.source_event,
        enabled: input.config.enabled,
        version: 0,
        config: clone(input.config),
        patched_fields: [],
        installed: input.config.enabled,
        phase: "stop",
        sensitivity: "sensitive",
        high_frequency: false,
        status: "stable",
        available: true,
        input_fields: [],
      };
      const index = this.rulesState.findIndex(
        (r) => r.agent === row.agent && r.source_event === row.source_event,
      );
      if (index >= 0) {
        this.rulesState[index] = row;
      } else {
        this.rulesState.push(row);
      }
      return clone(row);
    });
    this.saveProjectRulePatch = vi.fn(async (input): Promise<void> => {
      const key = patchKey(input.project_id, input.agent, input.source_event);
      const existing = this.patchesState.get(key) ?? {};
      const merged: Partial<RuleConfig> = { ...existing };
      for (const [field, value] of Object.entries(input.patch)) {
        if (value === undefined) {
          continue;
        }
        (merged as Record<string, unknown>)[field] = clone(value);
      }
      this.patchesState.set(key, merged);
    });
    this.resetProjectRuleField = vi.fn(
      async (input: Parameters<Backend["resetProjectRuleField"]>[0]): Promise<void> => {
        const key = patchKey(input.project_id, input.agent, input.source_event);
        const existing = this.patchesState.get(key);
        if (!existing) {
          return;
        }
        const { [input.field]: _removed, ...rest } = existing;
        void _removed;
        if (Object.keys(rest).length === 0) {
          this.patchesState.delete(key);
        } else {
          this.patchesState.set(key, rest);
        }
      },
    );
    this.previewNotification = vi.fn(async () => {
      if (this.opts.previewError) {
        throw new Error(this.opts.previewError);
      }
      return {
        title: "预览：Stop",
        severity: "info" as const,
        facts: [["事件", "Stop"] as [string, string]],
        body: this.opts.previewBody ?? "",
        footer: null,
      };
    });

    // Channels: mirrors commands::channels — official-host validation BEFORE
    // any credential would be stored; delete refuses targeted channels.
    this.saveChannel = vi.fn(async (input: SaveChannelInput): Promise<ChannelSummary> => {
      assertOfficialWebhook(input.credential);
      const saved: ChannelSummary = {
        id: `channel-${this.nextChannelId++}` as ChannelId,
        kind: input.credential.kind,
        name: input.name,
        credential_present: true,
        health: "unknown",
        paused: false,
        last_succeeded_at: null,
      };
      this.channelsState.push(saved);
      return clone(saved);
    });
    this.replaceChannelCredential = vi.fn(
      async (input: ReplaceChannelCredentialInput): Promise<ChannelSummary> => {
        const existing = this.channelsState.find((c) => c.id === input.channel_id);
        if (!existing) throw new Error("unknown channel");
        assertOfficialWebhook(input.credential);
        if (input.credential.kind !== existing.kind) {
          throw {
            code: "configuration.channel_kind_mismatch",
            message: "credential kind does not match channel",
          };
        }
        existing.health = "unknown";
        return clone(existing);
      },
    );
    this.deleteChannel = vi.fn(async (input: DeleteChannelInput): Promise<void> => {
      if (this.channelInUseIds.includes(input.channel_id)) {
        throw {
          code: "configuration.channel_in_use",
          message: "channel is targeted by an enabled rule",
          suggested_action: "先把指向该渠道的规则改投其他渠道，再删除。",
        };
      }
      this.channelsState = this.channelsState.filter((c) => c.id !== input.channel_id);
    });
    this.testChannel = vi.fn(
      async (_input: TestChannelInput): Promise<{ http_status: number; platform_code: string | null }> => {
        void _input;
        if (this.testChannelError) {
          throw { ...this.testChannelError };
        }
        return { ...this.testChannelResult };
      },
    );

    // Projects: mirrors configuration.path_conflict enforcement at the repo
    // layer for duplicate/overlapping registrations.
    this.saveProject = vi.fn(async (input: SaveProjectInput): Promise<ProjectSummary> => {
      if (this.projectConflict) {
        throw {
          code: "configuration.path_conflict",
          message: "project path is already registered",
        };
      }
      const saved: ProjectSummary = {
        id: input.project_id ?? (`project-${this.projectsState.length + 1}` as ProjectId),
        name: input.name,
        canonical_root: input.canonical_root,
        worktree_mode: input.worktree_mode,
        paths: [],
        override_count: 0,
      };
      this.projectsState.push(saved);
      return clone(saved);
    });
    this.addProjectAlias = vi.fn(async (input: AddProjectAliasInput): Promise<void> => {
      if (this.projectConflict) {
        throw {
          code: "configuration.path_conflict",
          message: "project path is already registered",
        };
      }
      const owner = this.projectsState.find((p) => p.id === input.project_id);
      owner?.paths.push({
        id: `path-${this.nextPathId++}`,
        kind: "alias",
        canonical_path: input.canonical_path,
      });
    });
    this.removeProjectAlias = vi.fn(async (input: RemoveProjectAliasInput): Promise<void> => {
      for (const project of this.projectsState) {
        project.paths = project.paths.filter((path) => path.id !== input.path_id);
      }
    });

    // History: real filtering + bounded pagination over the fixture rows, so
    // offset/next_offset behavior matches the core's page semantics.
    this.listHistory = vi.fn(async (input: ListHistoryInput = {}): Promise<HistoryPage> => {
      if (this.historyListError) {
        throw { ...this.historyListError };
      }
      const limit = input.limit ?? 50;
      const offset = input.offset ?? 0;
      const filtered = this.historyItemsState.filter((item) => {
        if (
          input.delivery_status !== undefined &&
          input.delivery_status !== null &&
          item.delivery_status !== input.delivery_status
        ) {
          return false;
        }
        if (input.project_id && item.project_id !== input.project_id) return false;
        if (input.source_event && item.source_event !== input.source_event) return false;
        if (input.channel_id && item.channel_id !== input.channel_id) return false;
        if (input.occurred_from && item.occurred_at < input.occurred_from) return false;
        if (input.occurred_until && item.occurred_at > input.occurred_until) return false;
        return true;
      });
      const slice = filtered.slice(offset, offset + limit);
      const next = offset + slice.length < filtered.length ? offset + slice.length : null;
      return { items: clone(slice), next_offset: next };
    });
    this.getHistoryDetail = vi.fn(
      async (input: GetHistoryDetailInput): Promise<HistoryPage> => {
        const found = this.historyItemsState.find(
          (item) => item.event_id === input.event_id,
        );
        if (!found) {
          throw { code: "history.event_not_found", message: "event not found" };
        }
        return { items: [clone(found)], next_offset: null };
      },
    );
    this.manualRetryDelivery = vi.fn(async (input: ManualRetryInput): Promise<void> => {
      if (this.retryError) {
        throw { ...this.retryError };
      }
      void input;
    });
    this.getSettings = vi.fn(async (): Promise<SettingsView> => clone(this.settings));
    this.setNotificationPause = vi.fn(
      async (input: SetPauseInput): Promise<SettingsView> => {
        const now = new Date();
        let until: Date;
        if (input.duration === "fifteen_minutes") {
          until = new Date(now.getTime() + 15 * 60_000);
        } else if (input.duration === "one_hour") {
          until = new Date(now.getTime() + 60 * 60_000);
        } else {
          until = new Date(now);
          until.setHours(24, 0, 0, 0); // local end of today
        }
        this.settings.paused_until = until.toISOString();
        return clone(this.settings);
      },
    );
    this.clearNotificationPause = vi.fn(async (): Promise<SettingsView> => {
      this.settings.paused_until = null;
      return clone(this.settings);
    });
    this.checkForUpdates = vi.fn(
      async (): Promise<UpdateCheckResult> => clone(this.updateCheckResult),
    );
    this.installUpdate = vi.fn(async (input: InstallUpdateInput): Promise<void> => {
      if (this.installUpdateError) {
        throw { ...this.installUpdateError };
      }
      void input;
    });
  }

  /** Last input passed to saveSettings (for persistence assertions). */
  get savedSettings(): readonly SaveSettingsInput[] {
    return this.savedSettingsInputs;
  }

  private snapshot(): HealthSnapshot {
    const snap = okSnapshot();
    if (this.healthOverride) {
      Object.assign(snap, clone(this.healthOverride));
    }
    snap.pending_jobs = Math.max(snap.pending_jobs, 0);
    if (this.selectionOutOfDate) {
      snap.issues.push({
        issue_code: "hooks.selection_out_of_date",
        level: "warning",
        message: "",
        suggested_command: null,
        suggested_action: null,
      });
      snap.overall = "warning";
    }
    return snap;
  }

  /** Test-only: simulate a core event push. Only the revision is honoured. */
  emit(event: CoreEventName, payload: { revision: number } & Record<string, unknown>): void {
    for (const handler of this.handlers.get(event) ?? []) {
      handler(payload.revision);
    }
  }

  async getBootstrapState(): Promise<BootstrapState> {
    const snap = this.snapshot();
    return {
      onboarding_completed: this.settings.onboarding_completed,
      locale: this.settings.locale,
      theme: this.settings.theme,
      health: snap,
      pending_jobs: snap.pending_jobs,
      failed_jobs: snap.failed_jobs,
    };
  }

  async listAgentIntegrations(): Promise<AgentIntegrationSummary[]> {
    return clone(this.opts.detectResults?.() ?? []);
  }

  async listHookRules(input: ListHookRulesInput): Promise<HookRuleRow[]> {
    const rows = clone(this.rulesState).filter((row) => row.agent === input.agent);
    if (!input.project_id) {
      return rows;
    }
    return rows.map((row) => {
      const patch = this.patchesState.get(
        patchKey(input.project_id as string, row.agent, row.source_event),
      );
      if (!patch) {
        return row;
      }
      const defined = Object.entries(patch).filter(([, value]) => value !== undefined);
      const patchedFields = defined.map(([field]) => field) as PatchFieldCode[];
      const config = { ...row.config };
      for (const [field, value] of defined) {
        (config as Record<string, unknown>)[field] = value;
      }
      return { ...row, enabled: config.enabled, config, patched_fields: patchedFields };
    });
  }

  async listChannels(): Promise<ChannelSummary[]> {
    return clone(this.channelsState);
  }

  async listProjects(): Promise<ProjectSummary[]> {
    return clone(this.projectsState);
  }

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
    return Promise.resolve(() => {
      set.delete(handler);
    });
  }
}

/** Fully-configured install: onboarding done, Chinese, system theme. */
export function configuredBackend(options: FakeBackendOptions = {}): FakeBackend {
  return new FakeBackend({
    onboardingCompleted: true,
    locale: "zh_cn",
    theme: "system",
    ...options,
  });
}

/** Fresh install: onboarding not completed, both agents detected cleanly. */
export function onboardingBackend(options: FakeBackendOptions = {}): FakeBackend {
  return new FakeBackend({
    onboardingCompleted: false,
    locale: "zh_cn",
    theme: "system",
    ...options,
  });
}

/** Codex hook trust is still pending: the checklist must block, not bypass. */
export function backendNeedingCodexTrust(): FakeBackend {
  return new FakeBackend({
    onboardingCompleted: false,
    locale: "zh_cn",
    theme: "system",
    detectResults: () => [
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
        needs_compatible_version_confirmation: true,
      },
    ],
  });
}

export function TestApp({ backend }: { backend: Backend }): ReactNode {
  return (
    <BackendProvider backend={backend}>
      <AppRoot />
    </BackendProvider>
  );
}
