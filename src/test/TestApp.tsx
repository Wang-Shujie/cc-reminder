// Deterministic in-memory Backend fake + render harness shared by the
// frontend tests. Production code never imports this module.
import { BackendProvider } from "../lib/backend";
import { AppRoot } from "../App";
import type { Backend } from "../lib/backend";
import type {
  AgentIntegrationSummary,
  BootstrapState,
  ChannelCredentialInput,
  ChannelId,
  ChannelSummary,
  CoreEventName,
  HealthSnapshot,
  HistoryPage,
  HookInstallationResult,
  HookRuleRow,
  LocaleCode,
  ProjectId,
  ProjectSummary,
  SaveChannelInput,
  SaveSettingsInput,
  SettingsView,
  ThemeCode,
} from "../lib/contracts";
import type { ReactNode } from "react";

function clone<T>(value: T): T {
  return structuredClone(value);
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
}

export class FakeBackend implements Backend {
  readonly handlers = new Map<CoreEventName, Set<(revision: number) => void>>();
  readonly getHealthSnapshot: Backend["getHealthSnapshot"];
  readonly saveSettings: Backend["saveSettings"];
  readonly sendRuleTest: Backend["sendRuleTest"];
  readonly detectAgents: Backend["detectAgents"];

  private opts: Required<Omit<FakeBackendOptions, "detectResults" | "channels">> &
    Pick<FakeBackendOptions, "detectResults">;
  private channelsState: ChannelSummary[];
  private rulesState: HookRuleRow[];
  private projectsState: ProjectSummary[];
  private settings: SettingsView;
  private savedSettingsInputs: SaveSettingsInput[] = [];
  private nextChannelId = 1;

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
    };
    this.channelsState = options.channels ? clone(options.channels) : [];
    this.rulesState = [];
    this.projectsState = [];
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
    this.getHealthSnapshot = vi.fn(async (): Promise<HealthSnapshot> =>
      clone(this.snapshot()),
    );
    this.saveSettings = vi.fn(async (input: SaveSettingsInput): Promise<SettingsView> => {
      this.savedSettingsInputs.push(clone(input));
      this.settings = { ...this.settings, ...input };
      return clone(this.settings);
    });
    this.sendRuleTest = vi.fn(async (): Promise<void> => undefined);
    this.detectAgents = vi.fn(
      async (_input: { confirm_compatible_version: boolean }): Promise<AgentIntegrationSummary[]> =>
        clone(this.opts.detectResults?.() ?? []),
    );
  }

  /** Last input passed to saveSettings (for persistence assertions). */
  get savedSettings(): readonly SaveSettingsInput[] {
    return this.savedSettingsInputs;
  }

  private snapshot(): HealthSnapshot {
    const snap = okSnapshot();
    snap.pending_jobs = Math.max(snap.pending_jobs, 0);
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

  async applyHookAction(): Promise<HookInstallationResult> {
    const entries: HookInstallationResult["entries"] = [];
    return { agent: "claude-code", selection_out_of_date: false, entries };
  }

  async listHookRules(): Promise<HookRuleRow[]> {
    return clone(this.rulesState);
  }

  async saveGlobalRule(input: Parameters<Backend["saveGlobalRule"]>[0]): Promise<HookRuleRow> {
    const row: HookRuleRow = {
      agent: input.agent,
      source_event: input.source_event,
      enabled: input.config.enabled,
      version: 0,
      config: clone(input.config),
    };
    this.rulesState.push(row);
    return clone(row);
  }

  async saveProjectRulePatch(): Promise<void> {}
  async resetProjectRuleField(): Promise<void> {}

  async previewNotification() {
    return {
      title: "Preview",
      severity: "info" as const,
      facts: [] as [string, string][],
      body: "",
      footer: null,
    };
  }

  async listChannels(): Promise<ChannelSummary[]> {
    return clone(this.channelsState);
  }

  async saveChannel(input: SaveChannelInput): Promise<ChannelSummary> {
    const kind = input.credential.kind;
    const saved: ChannelSummary = {
      id: `channel-${this.nextChannelId++}` as ChannelId,
      kind,
      name: input.name,
      credential_present: true,
      health: "unknown",
      paused: false,
      last_succeeded_at: null,
    };
    this.channelsState.push(saved);
    return clone(saved);
  }

  async replaceChannelCredential(input: {
    channel_id: ChannelId;
    credential: ChannelCredentialInput;
  }): Promise<ChannelSummary> {
    const existing = this.channelsState.find((c) => c.id === input.channel_id);
    if (!existing) throw new Error("unknown channel");
    existing.health = "unknown";
    return clone(existing);
  }

  async deleteChannel(input: { channel_id: ChannelId }): Promise<void> {
    this.channelsState = this.channelsState.filter((c) => c.id !== input.channel_id);
  }

  async testChannel(): Promise<{ http_status: number; platform_code: string | null }> {
    return { http_status: 200, platform_code: null };
  }

  async listProjects(): Promise<ProjectSummary[]> {
    return clone(this.projectsState);
  }

  async saveProject(input: {
    project_id: string | null;
    name: string;
    canonical_root: string;
    worktree_mode: "alias" | "separate";
  }): Promise<ProjectSummary> {
    const saved: ProjectSummary = {
      id: input.project_id ?? `project-${this.projectsState.length + 1}`,
      name: input.name,
      canonical_root: input.canonical_root,
      worktree_mode: input.worktree_mode,
      paths: [],
    };
    this.projectsState.push(saved);
    return clone(saved);
  }

  async addProjectAlias(): Promise<void> {}
  async removeProjectAlias(): Promise<void> {}

  async listHistory(): Promise<HistoryPage> {
    return { items: [], next_offset: null };
  }

  async getHistoryDetail(): Promise<HistoryPage> {
    return { items: [], next_offset: null };
  }

  async manualRetryDelivery(): Promise<void> {}

  async getSettings(): Promise<SettingsView> {
    return clone(this.settings);
  }

  async setNotificationPause(): Promise<SettingsView> {
    return clone(this.settings);
  }

  async clearNotificationPause(): Promise<SettingsView> {
    return clone(this.settings);
  }

  async checkForUpdates() {
    return { available: false, version: null, notes: null, installable: false };
  }

  async installUpdate(): Promise<void> {}

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
