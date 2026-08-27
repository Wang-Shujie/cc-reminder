// 规则命令的序列化视图与 build_rule_view 装饰(原样移出)。
use serde::Serialize;

use crate::events::catalog::{CapabilityStatus, HookCapability, Sensitivity};
use crate::model::RuleConfig;
use crate::rules::resolve::{StoredGlobalRule, StoredRulePatch, resolve_rule};
// ---------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct CapabilityFieldView {
    pub name: String,
    pub sensitivity: Sensitivity,
}

/// One row of the Hook Rules table: the stored global rule merged with its
/// project patch (when a project scope was requested) plus static capability
/// metadata from the embedded catalog. `available=false` marks events that are
/// catalogued but not supported by the detected agent version.
#[derive(Clone, Debug, Serialize)]
pub struct GlobalRuleView {
    pub agent: String,
    pub source_event: String,
    pub enabled: bool,
    pub version: u64,
    pub config: RuleConfig,
    /// Top-level RuleConfig fields present in the project patch (empty in
    /// global scope).
    pub patched_fields: Vec<String>,
    pub installed: bool,
    pub phase: String,
    pub sensitivity: Sensitivity,
    pub high_frequency: bool,
    pub status: CapabilityStatus,
    pub available: bool,
    pub input_fields: Vec<CapabilityFieldView>,
}

pub(super) fn build_rule_view(
    rule: StoredGlobalRule,
    patch: Option<&StoredRulePatch>,
    installed: bool,
    cap: Option<&HookCapability>,
    available: bool,
) -> GlobalRuleView {
    let config = match patch {
        Some(stored) => resolve_rule(&rule.config, Some(&stored.patch)),
        None => rule.config.clone(),
    };
    let patched_fields = patch
        .map(|stored| {
            let mut fields = Vec::new();
            if stored.patch.enabled.is_some() {
                fields.push("enabled".to_owned());
            }
            if stored.patch.targets.is_some() {
                fields.push("targets".to_owned());
            }
            if stored.patch.filters.is_some() {
                fields.push("filters".to_owned());
            }
            if stored.patch.privacy.is_some() {
                fields.push("privacy".to_owned());
            }
            if stored.patch.delivery.is_some() {
                fields.push("delivery".to_owned());
            }
            if stored.patch.quiet_hours.is_some() {
                fields.push("quiet_hours".to_owned());
            }
            fields
        })
        .unwrap_or_default();
    // Metadata comes from the reference catalog so unsupported rows still
    // render their catalog facts; legacy rows outside every catalog fall back
    // to conservative defaults.
    let (phase, sensitivity, high_frequency, status, input_fields) = match cap {
        Some(hook) => (
            hook.phase.clone(),
            hook.sensitivity,
            hook.high_frequency,
            hook.status,
            hook.input_fields
                .iter()
                .map(|field| CapabilityFieldView {
                    name: field.name.clone(),
                    sensitivity: field.sensitivity,
                })
                .collect(),
        ),
        None => (
            "unknown".to_owned(),
            Sensitivity::Sensitive,
            false,
            CapabilityStatus::Stable,
            Vec::new(),
        ),
    };
    GlobalRuleView {
        agent: rule.agent.as_str().into(),
        source_event: rule.source_event,
        enabled: config.enabled,
        version: rule.version,
        config,
        patched_fields,
        installed,
        phase,
        sensitivity,
        high_frequency,
        status,
        available,
        input_fields,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectionOutOfDateView {
    /// True when the rules-derived `required_hook_selection` differs from the
    /// installed hook rows. The frontend surfaces a "repair needed" badge.
    pub selection_out_of_date: bool,
    pub health_revision: u64,
}
