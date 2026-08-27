// 规则命令的 wire 输入 DTO(自 commands/rules.rs 原样移出,架构提案 §3)。
use serde::Deserialize;

use crate::model::{AgentKind, PatchField, RuleConfig};

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKindInput {
    ClaudeCode,
    Codex,
}
impl AgentKindInput {
    pub(super) fn into_kind(self) -> AgentKind {
        match self {
            Self::ClaudeCode => AgentKind::ClaudeCode,
            Self::Codex => AgentKind::Codex,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListHookRulesInput {
    pub agent: AgentKindInput,
    /// Absent/null → global-scope rows; otherwise effective rows for the
    /// project (global config merged with the stored patch).
    pub project_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveGlobalRuleInput {
    pub agent: AgentKindInput,
    pub source_event: String,
    pub config: RuleConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveProjectRulePatchInput {
    pub project_id: String,
    pub agent: AgentKindInput,
    pub source_event: String,
    pub patch: crate::model::RulePatch,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetProjectRuleFieldInput {
    pub project_id: String,
    pub agent: AgentKindInput,
    pub source_event: String,
    pub field: PatchFieldInput,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchFieldInput {
    Enabled,
    Targets,
    Filters,
    Privacy,
    Delivery,
    QuietHours,
}
impl PatchFieldInput {
    pub(super) fn into_field(self) -> PatchField {
        match self {
            Self::Enabled => PatchField::Enabled,
            Self::Targets => PatchField::Targets,
            Self::Filters => PatchField::Filters,
            Self::Privacy => PatchField::Privacy,
            Self::Delivery => PatchField::Delivery,
            Self::QuietHours => PatchField::QuietHours,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewNotificationInput {
    pub agent: AgentKindInput,
    pub source_event: String,
    /// Optional project scope; preview uses global rule if absent.
    pub project_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendRuleTestInput {
    pub agent: AgentKindInput,
    pub source_event: String,
    pub channel_id: String,
}
