// Typed mirrors of the Rust command surface (Task 15). Field names are the
// SERIALIZED serde names — change them together with `src-tauri/src/commands/*`.
// Read models never carry credential material; credentials appear only in
// write inputs.

declare const projectBrand: unique symbol;
declare const channelBrand: unique symbol;
declare const ruleBrand: unique symbol;

// Optional-brand aliases: opaque in intent, but plain string literals remain
// assignable so tests and object literals stay ergonomic.
export type ProjectId = string & { readonly [projectBrand]?: "ProjectId" };
export type ChannelId = string & { readonly [channelBrand]?: "ChannelId" };
export type RuleId = string & { readonly [ruleBrand]?: "RuleId" };

export type AgentKindCode = "claude-code" | "codex";
export type ChannelKindCode = "ding_talk" | "we_com";
export type LocaleCode = "zh_cn" | "en";
export type ThemeCode = "system" | "light" | "dark";
export type SeverityCode = "info" | "warning" | "error" | "critical";
export type HealthLevelCode = "ok" | "warning" | "error";
export type TrustStatusCode =
  | "not_required"
  | "needs_user_confirmation"
  | "observed_working";
export type InstallationHealthCode = "unknown" | "healthy" | "needs_repair" | "error";
export type DeliveryStatusCode =
  | "not_queued"
  | "pending"
  | "sending"
  | "retry_wait"
  | "succeeded"
  | "failed"
  | "expired";

export const CORE_EVENTS = [
  "core://health-changed",
  "core://queue-changed",
  "core://history-changed",
] as const;
export type CoreEventName = (typeof CORE_EVENTS)[number];

// ---------------------------------------------------------------------------
// Bootstrap + health
// ---------------------------------------------------------------------------

export interface HealthIssue {
  issue_code: string;
  level: HealthLevelCode;
  suggested_command: string | null;
  suggested_action: string | null;
}

export interface AgentIntegrationHealth {
  agent: string;
  health: string;
}

export interface ChannelHealthState {
  channel_id: string;
  name: string;
  health: string;
  paused: boolean;
}

export interface HealthSnapshot {
  overall: HealthLevelCode;
  agents: AgentIntegrationHealth[];
  channels: ChannelHealthState[];
  pending_jobs: number;
  retry_jobs: number;
  failed_jobs: number;
  expired_jobs: number;
  spool_count: number;
  rejected_count: number;
  last_success_at: string | null;
  issues: HealthIssue[];
}

export interface BootstrapState {
  onboarding_completed: boolean;
  locale: LocaleCode;
  theme: ThemeCode;
  health: HealthSnapshot;
  pending_jobs: number;
  failed_jobs: number;
}

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------

/** Mirrors commands::agents::AgentIntegrationView (Serialize). */
export interface AgentIntegrationSummary {
  agent: string;
  installed: boolean;
  version: string | null;
  executable_path: string | null;
  health: string;
  needs_compatible_version_confirmation: boolean;
}

export type HookActionCode = "install" | "repair" | "upgrade_helper" | "uninstall";

export interface ApplyHookActionInput {
  agent: AgentKindCode;
  action: HookActionCode;
  expected_health_revision: number;
  confirm_compatible_version: boolean;
}

export interface HookApplyEntry {
  source_event: string;
  trust_status: TrustStatusCode;
  health: InstallationHealthCode;
}

export interface HookInstallationResult {
  agent: string;
  selection_out_of_date: boolean;
  entries: HookApplyEntry[];
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

export interface TargetConfig {
  channel_id: ChannelId;
  template: string | null;
}

export interface FilterGroup {
  tool_names: string[];
  event_subtypes: string[];
  permission_modes: string[];
  models: string[];
  statuses: string[];
}

export type SummaryModeCode = "metadata_only" | "native_summary";

export interface PrivacyPolicy {
  allowed_sensitive_fields: string[];
  max_body_chars: number;
  summary_mode: SummaryModeCode;
  extra_redaction_patterns: string[];
}

export type DeliveryMode =
  | { mode: "immediate" }
  | { mode: "aggregate"; window_seconds: number };

export type QuietBehaviorCode = "suppress" | "defer";

export interface DeliveryPolicy {
  mode: DeliveryMode;
  cooldown_seconds: number;
  max_per_window: number;
  window_seconds: number;
  quiet_behavior: QuietBehaviorCode;
  ttl_seconds: number;
  max_attempts: number;
}

export interface QuietHours {
  start_local: string;
  end_local: string;
  weekdays: number[];
  bypass_at_or_above: SeverityCode | null;
}

export interface RuleConfig {
  enabled: boolean;
  targets: TargetConfig[];
  filters: FilterGroup;
  privacy: PrivacyPolicy;
  delivery: DeliveryPolicy;
  quiet_hours: QuietHours | null;
}

export interface HookRuleRow {
  agent: AgentKindCode;
  source_event: string;
  enabled: boolean;
  version: number;
  config: RuleConfig;
}

export interface ListHookRulesInput {
  agent: AgentKindCode;
}

export interface SaveGlobalRuleInput {
  agent: AgentKindCode;
  source_event: string;
  config: RuleConfig;
}

export type PatchFieldCode =
  | "enabled"
  | "targets"
  | "filters"
  | "privacy"
  | "delivery"
  | "quiet_hours";

export interface SaveProjectRulePatchInput {
  project_id: ProjectId;
  agent: AgentKindCode;
  source_event: string;
  patch: Partial<RuleConfig>;
}

export interface ResetProjectRuleFieldInput {
  project_id: ProjectId;
  agent: AgentKindCode;
  source_event: string;
  field: PatchFieldCode;
}

export interface PreviewNotificationInput {
  agent: AgentKindCode;
  source_event: string;
  project_id: ProjectId | null;
}

export interface NotificationDocument {
  title: string;
  severity: SeverityCode;
  facts: [string, string][];
  body: string;
  footer: string | null;
}

export interface SendRuleTestInput {
  agent: AgentKindCode;
  source_event: string;
  channel_id: ChannelId;
}

// ---------------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------------

export interface ChannelSummary {
  id: ChannelId;
  kind: ChannelKindCode;
  name: string;
  credential_present: boolean;
  /** Serialized health summary string from the core (never parsed here). */
  health: string;
  paused: boolean;
  last_succeeded_at: string | null;
}

export type ChannelCredentialInput =
  | { kind: "ding_talk"; webhook: string; signing_secret?: string | null }
  | { kind: "we_com"; webhook: string };

export interface SaveChannelInput {
  channel_id: ChannelId | null;
  name: string;
  keyword_prefix?: string | null;
  credential: ChannelCredentialInput;
}

export interface ReplaceChannelCredentialInput {
  channel_id: ChannelId;
  credential: ChannelCredentialInput;
}

export interface DeleteChannelInput {
  channel_id: ChannelId;
}

export interface TestChannelInput {
  channel_id: ChannelId;
}

export interface DeliveryReceiptDto {
  http_status: number;
  platform_code: string | null;
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

export type WorktreeModeCode = "alias" | "separate";

export interface ProjectSummary {
  id: ProjectId;
  name: string;
  canonical_root: string;
  worktree_mode: WorktreeModeCode;
  paths: { path: string; kind: string }[];
}

export interface SaveProjectInput {
  project_id: ProjectId | null;
  name: string;
  canonical_root: string;
  worktree_mode: WorktreeModeCode;
}

export interface AddProjectAliasInput {
  project_id: ProjectId;
  path: string;
}

export interface RemoveProjectAliasInput {
  project_id: ProjectId;
  path: string;
}

// ---------------------------------------------------------------------------
// History + delivery jobs
// ---------------------------------------------------------------------------

export interface DeliveryAttemptDto {
  attempt_number: number;
  started_at: string;
  completed_at: string;
  outcome: string;
  http_status: number | null;
  platform_code: string | null;
  error_code: string | null;
  retry_at: string | null;
  redacted_detail: string | null;
}

export interface HistoryItem {
  event_id: string;
  source: AgentKindCode;
  source_version: string;
  source_event: string;
  category: string;
  occurred_at: string;
  received_at: string;
  project_id: ProjectId | null;
  project_display_name: string | null;
  unmatched_cwd_fingerprint: string | null;
  model: string | null;
  permission_mode: string | null;
  severity: SeverityCode;
  public_fields: Record<string, string | number | boolean | null>;
  correlation_id: string;
  processing_outcome: string;
  outcome_reason_code: string | null;
  delivery_job_id: string | null;
  channel_id: ChannelId | null;
  document: NotificationDocument | null;
  delivery_status: DeliveryStatusCode;
  attempts: DeliveryAttemptDto[];
}

export interface HistoryPage {
  items: HistoryItem[];
  next_offset: number | null;
}

export interface ListHistoryInput {
  occurred_from?: string | null;
  occurred_until?: string | null;
  project_id?: ProjectId | null;
  source?: AgentKindCode | null;
  source_event?: string | null;
  channel_id?: ChannelId | null;
  delivery_status?: DeliveryStatusCode | null;
  offset?: number;
  limit?: number;
}

export interface GetHistoryDetailInput {
  event_id: string;
}

export interface ManualRetryInput {
  job_id: string;
}

export interface DeliveryJobSummary {
  job_id: string;
  state: DeliveryStatusCode;
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

export interface SettingsView {
  autostart: boolean;
  close_to_tray: boolean;
  locale: LocaleCode;
  theme: ThemeCode;
  event_retention_days: number;
  log_retention_days: number;
  onboarding_completed: boolean;
  paused_until: string | null;
}

export interface SaveSettingsInput {
  autostart: boolean;
  close_to_tray: boolean;
  locale: LocaleCode;
  theme: ThemeCode;
  event_retention_days: number;
  log_retention_days: number;
  onboarding_completed: boolean;
}

export type PauseDurationCode = "fifteen_minutes" | "one_hour" | "today";

export interface SetPauseInput {
  duration: PauseDurationCode;
}

// ---------------------------------------------------------------------------
// Updates
// ---------------------------------------------------------------------------

export interface UpdateCheckResult {
  available: boolean;
  version: string | null;
  notes: string | null;
  installable: boolean;
}

export interface InstallUpdateInput {
  confirmed: boolean;
}
