use std::collections::BTreeMap;

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

fn deserialize_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
