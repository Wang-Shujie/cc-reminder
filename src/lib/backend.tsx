// Typed boundary to the Rust core. TauriBackend is the ONLY module that may
// import @tauri-apps/api; everything else consumes the Backend interface so
// tests can inject the deterministic in-memory fake.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { createContext, useContext, type ReactNode } from "react";

import type {
  AddProjectAliasInput,
  AgentIntegrationSummary,
  ApplyHookActionInput,
  BootstrapState,
  ChannelSummary,
  CoreEventName,
  DeleteChannelInput,
  DeliveryReceiptDto,
  DiagnosticExportResult,
  GetHistoryDetailInput,
  HealthSnapshot,
  HistoryPage,
  HookInstallationResult,
  HookRuleRow,
  InstallUpdateInput,
  ListHookRulesInput,
  ListHistoryInput,
  ManualRetryInput,
  NotificationDocument,
  PreviewNotificationInput,
  ProjectSummary,
  RemoveProjectAliasInput,
  ReplaceChannelCredentialInput,
  ResetProjectRuleFieldInput,
  SaveChannelInput,
  SaveGlobalRuleInput,
  SaveProjectRulePatchInput,
  SaveProjectInput,
  SaveSettingsInput,
  SendRuleTestInput,
  SetDebugLoggingInput,
  SetPauseInput,
  SettingsView,
  TestChannelInput,
  UpdateCheckResult,
} from "./contracts";

export interface Backend {
  /** `offsetSeconds` is this side's UTC offset (-Date#getTimezoneOffset()*60,
   *  east-positive). The core persists it at first paint so quiet hours run in
   *  local time — same frontend-reported pattern as setNotificationPause. */
  getBootstrapState(offsetSeconds?: number | null): Promise<BootstrapState>;
  getHealthSnapshot(): Promise<HealthSnapshot>;
  detectAgents(input: { confirm_compatible_version: boolean }): Promise<AgentIntegrationSummary[]>;
  listAgentIntegrations(): Promise<AgentIntegrationSummary[]>;
  applyHookAction(input: ApplyHookActionInput): Promise<HookInstallationResult>;
  listHookRules(input: ListHookRulesInput): Promise<HookRuleRow[]>;
  saveGlobalRule(input: SaveGlobalRuleInput): Promise<HookRuleRow>;
  saveProjectRulePatch(input: SaveProjectRulePatchInput): Promise<void>;
  resetProjectRuleField(input: ResetProjectRuleFieldInput): Promise<void>;
  previewNotification(input: PreviewNotificationInput): Promise<NotificationDocument>;
  sendRuleTest(input: SendRuleTestInput): Promise<void>;
  listChannels(): Promise<ChannelSummary[]>;
  saveChannel(input: SaveChannelInput): Promise<ChannelSummary>;
  replaceChannelCredential(
    input: ReplaceChannelCredentialInput,
  ): Promise<ChannelSummary>;
  deleteChannel(input: DeleteChannelInput): Promise<void>;
  testChannel(input: TestChannelInput): Promise<DeliveryReceiptDto>;
  listProjects(): Promise<ProjectSummary[]>;
  saveProject(input: SaveProjectInput): Promise<ProjectSummary>;
  addProjectAlias(input: AddProjectAliasInput): Promise<void>;
  removeProjectAlias(input: RemoveProjectAliasInput): Promise<void>;
  listHistory(input?: ListHistoryInput): Promise<HistoryPage>;
  getHistoryDetail(input: GetHistoryDetailInput): Promise<HistoryPage>;
  manualRetryDelivery(input: ManualRetryInput): Promise<void>;
  getSettings(): Promise<SettingsView>;
  saveSettings(input: SaveSettingsInput): Promise<SettingsView>;
  setNotificationPause(input: SetPauseInput): Promise<SettingsView>;
  clearNotificationPause(): Promise<SettingsView>;
  checkForUpdates(): Promise<UpdateCheckResult>;
  installUpdate(input: InstallUpdateInput): Promise<void>;
  /** Opens the native save dialog in the core (Rust); no path crosses the
   *  bridge from this side. */
  exportDiagnostics(): Promise<DiagnosticExportResult>;
  clearHistory(input: { preserve_active_jobs: boolean }): Promise<number>;
  setDebugLogging(input: SetDebugLoggingInput): Promise<SettingsView>;
  subscribe(
    event: CoreEventName,
    handler: (revision: number) => void,
  ): Promise<() => void>;
}

/** Production Backend: one hardcoded invoke name per command. */
export class TauriBackend implements Backend {
  getBootstrapState(offsetSeconds?: number | null): Promise<BootstrapState> {
    return invoke("get_bootstrap_state", {
      // Tauri command args are camelCase on this side of the bridge.
      offsetSeconds: offsetSeconds ?? null,
    });
  }
  getHealthSnapshot(): Promise<HealthSnapshot> {
    return invoke("get_health_snapshot");
  }
  detectAgents(input: {
    confirm_compatible_version: boolean;
  }): Promise<AgentIntegrationSummary[]> {
    return invoke("detect_agents", { input });
  }
  listAgentIntegrations(): Promise<AgentIntegrationSummary[]> {
    return invoke("list_agent_integrations");
  }
  applyHookAction(input: ApplyHookActionInput): Promise<HookInstallationResult> {
    return invoke("apply_hook_action", { input });
  }
  listHookRules(input: ListHookRulesInput): Promise<HookRuleRow[]> {
    return invoke("list_hook_rules", { input });
  }
  saveGlobalRule(input: SaveGlobalRuleInput): Promise<HookRuleRow> {
    return invoke("save_global_rule", { input });
  }
  saveProjectRulePatch(input: SaveProjectRulePatchInput): Promise<void> {
    return invoke("save_project_rule_patch", { input });
  }
  resetProjectRuleField(input: ResetProjectRuleFieldInput): Promise<void> {
    return invoke("reset_project_rule_field", { input });
  }
  previewNotification(input: PreviewNotificationInput): Promise<NotificationDocument> {
    return invoke("preview_notification", { input });
  }
  sendRuleTest(input: SendRuleTestInput): Promise<void> {
    return invoke("send_rule_test", { input });
  }
  listChannels(): Promise<ChannelSummary[]> {
    return invoke("list_channels");
  }
  saveChannel(input: SaveChannelInput): Promise<ChannelSummary> {
    return invoke("save_channel", { input });
  }
  replaceChannelCredential(
    input: ReplaceChannelCredentialInput,
  ): Promise<ChannelSummary> {
    return invoke("replace_channel_credential", { input });
  }
  deleteChannel(input: DeleteChannelInput): Promise<void> {
    return invoke("delete_channel", { input });
  }
  testChannel(input: TestChannelInput): Promise<DeliveryReceiptDto> {
    return invoke("test_channel", { input });
  }
  listProjects(): Promise<ProjectSummary[]> {
    return invoke("list_projects");
  }
  saveProject(input: SaveProjectInput): Promise<ProjectSummary> {
    return invoke("save_project", { input });
  }
  addProjectAlias(input: AddProjectAliasInput): Promise<void> {
    return invoke("add_project_alias", { input });
  }
  removeProjectAlias(input: RemoveProjectAliasInput): Promise<void> {
    return invoke("remove_project_alias", { input });
  }
  listHistory(input?: ListHistoryInput): Promise<HistoryPage> {
    return invoke("list_history", {
      filter: input ?? {},
      page: { offset: input?.offset ?? 0, limit: input?.limit ?? 50 },
    });
  }
  getHistoryDetail(input: GetHistoryDetailInput): Promise<HistoryPage> {
    return invoke("get_history_detail", { input });
  }
  manualRetryDelivery(input: ManualRetryInput): Promise<void> {
    return invoke("manual_retry_delivery", { input });
  }
  getSettings(): Promise<SettingsView> {
    return invoke("get_settings");
  }
  saveSettings(input: SaveSettingsInput): Promise<SettingsView> {
    return invoke("save_settings", { input });
  }
  setNotificationPause(input: SetPauseInput): Promise<SettingsView> {
    return invoke("set_notification_pause", {
      duration: input.duration,
      // Tauri command args are camelCase on this side of the bridge.
      offsetSeconds: input.offset_seconds ?? null,
    });
  }
  clearNotificationPause(): Promise<SettingsView> {
    return invoke("clear_notification_pause");
  }
  checkForUpdates(): Promise<UpdateCheckResult> {
    return invoke("check_for_updates");
  }
  installUpdate(input: InstallUpdateInput): Promise<void> {
    return invoke("install_update", { input });
  }
  exportDiagnostics(): Promise<DiagnosticExportResult> {
    return invoke("export_diagnostics");
  }
  clearHistory(input: { preserve_active_jobs: boolean }): Promise<number> {
    return invoke("clear_history", { input });
  }
  setDebugLogging(input: SetDebugLoggingInput): Promise<SettingsView> {
    return invoke("set_debug_logging", { input });
  }
  async subscribe(
    event: CoreEventName,
    handler: (revision: number) => void,
  ): Promise<() => void> {
    const unlisten: UnlistenFn = await listen(event, (payload) => {
      // Only the revision is honoured; payload details are never trusted.
      const data = payload.payload as { revision?: unknown };
      handler(typeof data.revision === "number" ? data.revision : 0);
    });
    return unlisten;
  }
}

const BackendContext = createContext<Backend | null>(null);

export function BackendProvider({
  backend,
  children,
}: {
  backend: Backend;
  children: ReactNode;
}): ReactNode {
  return <BackendContext.Provider value={backend}>{children}</BackendContext.Provider>;
}

export function useBackend(): Backend {
  const backend = useContext(BackendContext);
  if (!backend) {
    throw new Error("BackendProvider is missing above this component");
  }
  return backend;
}

/** Context lookup that tolerates a missing provider so management pages can
 *  take an injected `backend` prop in isolation (unit tests) while production
 *  wiring stays context-based. */
export function useOptionalBackend(): Backend | null {
  return useContext(BackendContext);
}

/** Stable stand-in that throws only when a method is actually invoked, so a
 *  page rendered without provider AND without injected prop fails loudly at
 *  first backend use instead of breaking hook order during render. */
const MISSING_BACKEND: Backend = new Proxy({} as Backend, {
  get() {
    throw new Error("BackendProvider is missing above this component");
  },
});

/** Backend resolution for management pages: injected prop wins (tests),
 *  otherwise context (production shell wiring). */
export function usePageBackend(injected?: Backend): Backend {
  const contextual = useOptionalBackend();
  return injected ?? contextual ?? MISSING_BACKEND;
}
