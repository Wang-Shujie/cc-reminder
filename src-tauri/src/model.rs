use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::actions::ActionCapability;

pub type ProjectId = Uuid;
pub type ChannelId = Uuid;
pub type RuleId = Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
}

impl AgentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    Session,
    Prompt,
    Tool,
    Permission,
    Compaction,
    Subagent,
    Task,
    Configuration,
    Worktree,
    Notification,
    Completion,
    Other,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ScalarValue {
    String(String),
    Number(f64),
    Bool(bool),
    Null,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EncryptedBlobRef {
    pub blob_id: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope {
    pub id: Uuid,
    pub source: AgentKind,
    pub source_version: Version,
    pub source_event: String,
    pub category: EventCategory,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub project_id: Option<ProjectId>,
    pub project_display_name: Option<String>,
    pub unmatched_cwd_fingerprint: Option<String>,
    pub session_ref: Option<String>,
    pub turn_ref: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub severity: Severity,
    pub public_fields: BTreeMap<String, ScalarValue>,
    pub encrypted_sensitive_fields: Option<EncryptedBlobRef>,
    pub correlation_id: Uuid,
    pub action_id: Option<String>,
    pub action_capabilities: Vec<ActionCapability>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NotificationDocument {
    pub title: String,
    pub severity: Severity,
    pub facts: Vec<(String, String)>,
    pub body: String,
    pub footer: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TargetConfig {
    pub channel_id: ChannelId,
    pub template: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FilterGroup {
    pub tool_names: Vec<String>,
    pub event_subtypes: Vec<String>,
    pub permission_modes: Vec<String>,
    pub models: Vec<String>,
    pub statuses: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrivacyPolicy {
    pub allowed_sensitive_fields: Vec<String>,
    pub max_body_chars: u32,
    pub summary_mode: SummaryMode,
    pub extra_redaction_patterns: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryMode {
    MetadataOnly,
    NativeSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DeliveryMode {
    Immediate,
    Aggregate { window_seconds: u32 },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuietBehavior {
    Suppress,
    Defer,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeliveryPolicy {
    pub mode: DeliveryMode,
    pub cooldown_seconds: u32,
    pub max_per_window: u32,
    pub window_seconds: u32,
    pub quiet_behavior: QuietBehavior,
    pub ttl_seconds: u32,
    pub max_attempts: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct QuietHours {
    pub start_local: String,
    pub end_local: String,
    pub weekdays: Vec<u8>,
    pub bypass_at_or_above: Option<Severity>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NotificationPause {
    pub started_at: DateTime<Utc>,
    pub until: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RuleConfig {
    pub enabled: bool,
    pub targets: Vec<TargetConfig>,
    pub filters: FilterGroup,
    pub privacy: PrivacyPolicy,
    pub delivery: DeliveryPolicy,
    pub quiet_hours: Option<QuietHours>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RulePatch {
    pub enabled: Option<bool>,
    pub targets: Option<Vec<TargetConfig>>,
    pub filters: Option<FilterGroup>,
    pub privacy: Option<PrivacyPolicy>,
    pub delivery: Option<DeliveryPolicy>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub quiet_hours: Option<Option<QuietHours>>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    DingTalk,
    WeCom,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelPublicConfig {
    DingTalk { keyword_prefix: Option<String> },
    WeCom,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelHealth {
    Unknown,
    Healthy,
    PausedAuthentication,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialAvailability {
    Available,
    Unavailable { reason_code: String },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustStatus {
    NotRequired,
    NeedsUserConfirmation,
    ObservedWorking,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallationHealth {
    Unknown,
    Healthy,
    NeedsRepair,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Locale {
    ZhCn,
    En,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PatchField {
    Enabled,
    Targets,
    Filters,
    Privacy,
    Delivery,
    QuietHours,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeMode {
    Alias,
    Separate,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectPathKind {
    Root,
    Alias,
    Worktree,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: ProjectId,
    pub name: String,
    pub canonical_root: PathBuf,
    pub worktree_mode: WorktreeMode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectPathRecord {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub canonical_path: PathBuf,
    pub kind: ProjectPathKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectMatchCacheProject {
    pub id: ProjectId,
    pub display_name: String,
    pub canonical_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectMatchCacheFile {
    pub version: u8,
    pub projects: Vec<ProjectMatchCacheProject>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectCacheHealth {
    Healthy,
    RegenerationFailed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelRecord {
    pub id: ChannelId,
    pub kind: ChannelKind,
    pub name: String,
    pub credential_ref: String,
    pub public_config: ChannelPublicConfig,
    pub health_status: ChannelHealth,
    pub paused_reason_code: Option<String>,
    pub consecutive_auth_failures: u8,
    pub last_succeeded_at: Option<DateTime<Utc>>,
    pub next_allowed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub autostart: bool,
    pub close_to_tray: bool,
    pub locale: Locale,
    pub theme: Theme,
    pub event_retention_days: u16,
    pub log_retention_days: u16,
    pub notification_pause: Option<NotificationPause>,
    pub debug_until: Option<DateTime<Utc>>,
    pub onboarding_completed: bool,
    /// The WebView's UTC offset in seconds (east-positive), reported at every
    /// bootstrap (`-Date#getTimezoneOffset()*60`) and persisted here — the same
    /// frontend-reported pattern as the Task 19 pause fix. Feeds quiet-hours
    /// evaluation in the live pipeline; chrono's own local-offset lookup is
    /// unavailable without its `clock` feature. `0` (= UTC) until the first
    /// report; `#[serde(default)]` keeps older persisted JSON loadable.
    #[serde(default)]
    pub local_offset_seconds: i32,
    /// 全局通知正文模板(用户裁决 2026-08-27:统一格式且可编辑)。`None` =
    /// 内建统一默认(标题/五项 facts/摘要正文)。变量语法同规则级模板;
    /// 每条规则的渠道级模板仍可覆盖此值。
    #[serde(default)]
    pub notification_template: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            autostart: false,
            close_to_tray: true,
            locale: Locale::ZhCn,
            theme: Theme::System,
            event_retention_days: 30,
            log_retention_days: 7,
            notification_pause: None,
            debug_until: None,
            onboarding_completed: false,
            local_offset_seconds: 0,
            notification_template: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentInstallationRecord {
    pub agent: AgentKind,
    pub executable_path: Option<PathBuf>,
    pub version: Option<Version>,
    pub capability_verification: crate::events::catalog::CatalogVerification,
    pub health_status: InstallationHealth,
    pub last_checked_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookInstallationRecord {
    pub agent: AgentKind,
    pub source_event: String,
    pub command_fingerprint: String,
    pub definition_fingerprint: String,
    pub helper_version: String,
    pub config_hash: String,
    pub trust_status: TrustStatus,
    pub health_status: InstallationHealth,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSnapshotRecord {
    pub id: Uuid,
    pub agent: AgentKind,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub aad: String,
    pub source_hash: String,
    pub file_mode: Option<u32>,
    pub created_at: DateTime<Utc>,
}

fn deserialize_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
