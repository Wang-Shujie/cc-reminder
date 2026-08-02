use std::collections::BTreeSet;

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, ErrorDomain};
use crate::events::catalog::catalogued_hooks;
use crate::model::{
    AgentKind, DeliveryMode, DeliveryPolicy, FilterGroup, PrivacyPolicy, ProjectId, QuietBehavior,
    RuleConfig, RuleId, RulePatch, SummaryMode,
};

const MAX_RULE_LIST_ITEMS: usize = 100;
const MAX_RULE_VALUE_BYTES: usize = 256;
const MAX_REDACTION_PATTERNS: usize = 32;
const MAX_REDACTION_PATTERN_CHARS: usize = 512;
const MAX_TEMPLATE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredGlobalRule {
    pub id: RuleId,
    pub agent: AgentKind,
    pub source_event: String,
    pub version: u64,
    pub config: RuleConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredRulePatch {
    pub project_id: ProjectId,
    pub agent: AgentKind,
    pub source_event: String,
    pub version: u64,
    pub patch: RulePatch,
}

#[derive(Clone, Debug)]
pub struct ResolvedRule {
    pub id: RuleId,
    pub version: String,
    pub config: RuleConfig,
}

pub fn resolve_rule(global: &RuleConfig, patch: Option<&RulePatch>) -> RuleConfig {
    let mut resolved = global.clone();
    let Some(patch) = patch else {
        return resolved;
    };

    if let Some(enabled) = patch.enabled {
        resolved.enabled = enabled;
    }
    if let Some(targets) = &patch.targets {
        resolved.targets.clone_from(targets);
    }
    if let Some(filters) = &patch.filters {
        resolved.filters.clone_from(filters);
    }
    if let Some(privacy) = &patch.privacy {
        resolved.privacy.clone_from(privacy);
    }
    if let Some(delivery) = &patch.delivery {
        resolved.delivery.clone_from(delivery);
    }
    if let Some(quiet_hours) = &patch.quiet_hours {
        resolved.quiet_hours.clone_from(quiet_hours);
    }

    resolved
}

pub fn resolve_stored_rule(
    global: &StoredGlobalRule,
    patch: Option<&StoredRulePatch>,
) -> ResolvedRule {
    let config = resolve_rule(&global.config, patch.map(|stored| &stored.patch));
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, global.id.as_bytes());
    hash_part(
        &mut hasher,
        patch
            .map(|stored| stored.project_id.as_bytes().as_slice())
            .unwrap_or_default(),
    );
    hash_part(
        &mut hasher,
        &serde_json::to_vec(&config).expect("RuleConfig serialization cannot fail"),
    );

    ResolvedRule {
        id: global.id,
        version: hex::encode(hasher.finalize()),
        config,
    }
}

pub fn required_hook_selection(
    global: &[StoredGlobalRule],
    overrides: &[StoredRulePatch],
) -> BTreeSet<(AgentKind, String)> {
    let catalogued = catalogued_hooks();
    global
        .iter()
        .filter(|rule| {
            rule.config.enabled && catalogued.contains(&(rule.agent, rule.source_event.clone()))
        })
        .map(|rule| (rule.agent, rule.source_event.clone()))
        .chain(
            overrides
                .iter()
                .filter(|rule| {
                    rule.patch.enabled == Some(true)
                        && catalogued.contains(&(rule.agent, rule.source_event.clone()))
                })
                .map(|rule| (rule.agent, rule.source_event.clone())),
        )
        .collect()
}

pub fn validate_rule(rule: &RuleConfig) -> Result<(), AppError> {
    let delivery = &rule.delivery;
    let aggregate_window_is_valid = match delivery.mode {
        DeliveryMode::Immediate => true,
        DeliveryMode::Aggregate { window_seconds } => (10..=3_600).contains(&window_seconds),
    };
    let quiet_hours_are_valid = rule.quiet_hours.as_ref().is_none_or(|quiet| {
        parse_quiet_time(&quiet.start_local).is_some()
            && parse_quiet_time(&quiet.end_local).is_some()
            && quiet.weekdays.iter().all(|day| (1..=7).contains(day))
    });
    let filter_values = [
        rule.filters.tool_names.as_slice(),
        rule.filters.event_subtypes.as_slice(),
        rule.filters.permission_modes.as_slice(),
        rule.filters.models.as_slice(),
        rule.filters.statuses.as_slice(),
    ];
    let bounded_strings = filter_values.into_iter().all(valid_rule_strings)
        && valid_rule_strings(&rule.privacy.allowed_sensitive_fields)
        && valid_redaction_patterns(&rule.privacy.extra_redaction_patterns)
        && rule.targets.iter().all(|target| {
            target
                .template
                .as_ref()
                .is_none_or(|template| template.len() <= MAX_TEMPLATE_BYTES)
        });

    if rule.targets.len() > 20
        || rule.privacy.max_body_chars > 4_000
        || !(1..=86_400).contains(&delivery.ttl_seconds)
        || !(1..=10).contains(&delivery.max_attempts)
        || !aggregate_window_is_valid
        || delivery.cooldown_seconds > 86_400
        || delivery.window_seconds > 86_400
        || !(1..=100).contains(&delivery.max_per_window)
        || !quiet_hours_are_valid
        || !bounded_strings
    {
        return Err(AppError {
            domain: ErrorDomain::Configuration,
            code: "rule_invalid".into(),
            message: "notification rule is invalid".into(),
            suggested_action: None,
        });
    }

    Ok(())
}

fn valid_rule_strings(values: &[String]) -> bool {
    values.len() <= MAX_RULE_LIST_ITEMS
        && values
            .iter()
            .all(|value| value.len() <= MAX_RULE_VALUE_BYTES)
}

fn valid_redaction_patterns(values: &[String]) -> bool {
    values.len() <= MAX_REDACTION_PATTERNS
        && values
            .iter()
            .all(|value| value.chars().count() <= MAX_REDACTION_PATTERN_CHARS)
}

pub fn default_rule(agent: AgentKind, event: &str) -> RuleConfig {
    let enabled = matches!(
        (agent, event),
        (
            AgentKind::ClaudeCode,
            "PermissionRequest" | "Notification" | "Stop" | "StopFailure"
        ) | (AgentKind::Codex, "PermissionRequest" | "Stop")
    );
    let native_summary = matches!(event, "Stop" | "StopFailure" | "PostToolUseFailure");

    RuleConfig {
        enabled,
        targets: Vec::new(),
        filters: FilterGroup::default(),
        privacy: PrivacyPolicy {
            allowed_sensitive_fields: Vec::new(),
            max_body_chars: if native_summary { 500 } else { 0 },
            summary_mode: if native_summary {
                SummaryMode::NativeSummary
            } else {
                SummaryMode::MetadataOnly
            },
            extra_redaction_patterns: Vec::new(),
        },
        delivery: DeliveryPolicy {
            mode: DeliveryMode::Immediate,
            cooldown_seconds: 0,
            max_per_window: 100,
            window_seconds: 3_600,
            quiet_behavior: QuietBehavior::Suppress,
            ttl_seconds: if event == "PermissionRequest" {
                600
            } else {
                1_800
            },
            max_attempts: 5,
        },
        quiet_hours: None,
    }
}

pub(crate) fn parse_quiet_time(value: &str) -> Option<NaiveTime> {
    let bytes = value.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes[..2].iter().chain(&bytes[3..]).all(u8::is_ascii_digit)
    {
        return None;
    }

    let hour = u32::from(bytes[0] - b'0') * 10 + u32::from(bytes[1] - b'0');
    let minute = u32::from(bytes[3] - b'0') * 10 + u32::from(bytes[4] - b'0');
    NaiveTime::from_hms_opt(hour, minute, 0)
}

fn hash_part(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        StoredGlobalRule, StoredRulePatch, default_rule, required_hook_selection, resolve_rule,
        resolve_stored_rule, validate_rule,
    };
    use crate::error::ErrorDomain;
    use crate::model::{
        AgentKind, DeliveryMode, DeliveryPolicy, FilterGroup, PrivacyPolicy, QuietBehavior,
        QuietHours, RuleConfig, RulePatch, SummaryMode, TargetConfig,
    };

    #[test]
    fn empty_targets_override_instead_of_inherit() {
        let global = enabled_rule_with_targets(vec![target(1), target(2)]);
        let patch = RulePatch {
            targets: Some(vec![]),
            ..RulePatch::default()
        };

        assert_eq!(resolve_rule(&global, Some(&patch)).targets, Vec::new());
    }

    #[test]
    fn double_option_can_clear_quiet_hours() {
        let global = rule_with_quiet_hours("22:00", "08:00");
        let patch = RulePatch {
            quiet_hours: Some(None),
            ..RulePatch::default()
        };

        assert_eq!(resolve_rule(&global, Some(&patch)).quiet_hours, None);
    }

    #[test]
    fn quiet_hours_json_distinguishes_missing_from_explicit_null() {
        let inherited: RulePatch = serde_json::from_value(json!({})).unwrap();
        let cleared: RulePatch = serde_json::from_value(json!({"quiet_hours": null})).unwrap();

        assert_eq!(inherited.quiet_hours, None);
        assert_eq!(cleared.quiet_hours, Some(None));
    }

    #[test]
    fn absent_patch_fields_inherit_the_complete_global_rule() {
        let global = rule_with_quiet_hours("22:00", "08:00");

        assert_eq!(resolve_rule(&global, Some(&RulePatch::default())), global);
        assert_eq!(resolve_rule(&global, None), global);
    }

    #[test]
    fn every_present_patch_field_replaces_its_global_field_atomically() {
        let global = rule_with_quiet_hours("22:00", "08:00");
        let replacement = RuleConfig {
            enabled: false,
            targets: vec![target(9)],
            filters: FilterGroup {
                tool_names: vec!["Read".into()],
                ..FilterGroup::default()
            },
            privacy: PrivacyPolicy {
                allowed_sensitive_fields: vec!["last_assistant_message".into()],
                max_body_chars: 321,
                summary_mode: SummaryMode::NativeSummary,
                extra_redaction_patterns: vec!["secret".into()],
            },
            delivery: DeliveryPolicy {
                mode: DeliveryMode::Aggregate { window_seconds: 60 },
                cooldown_seconds: 10,
                max_per_window: 3,
                window_seconds: 120,
                quiet_behavior: QuietBehavior::Defer,
                ttl_seconds: 900,
                max_attempts: 3,
            },
            quiet_hours: Some(QuietHours {
                start_local: "09:00".into(),
                end_local: "17:00".into(),
                weekdays: vec![1, 2, 3, 4, 5],
                bypass_at_or_above: None,
            }),
        };
        let patch = RulePatch {
            enabled: Some(replacement.enabled),
            targets: Some(replacement.targets.clone()),
            filters: Some(replacement.filters.clone()),
            privacy: Some(replacement.privacy.clone()),
            delivery: Some(replacement.delivery.clone()),
            quiet_hours: Some(replacement.quiet_hours.clone()),
        };

        assert_eq!(resolve_rule(&global, Some(&patch)), replacement);
    }

    #[test]
    fn project_disablement_overrides_a_globally_enabled_rule() {
        let global = enabled_rule_with_targets(vec![target(1)]);
        let patch = RulePatch {
            enabled: Some(false),
            ..RulePatch::default()
        };

        assert!(!resolve_rule(&global, Some(&patch)).enabled);
    }

    #[test]
    fn effective_version_is_hex_and_ignores_storage_revisions() {
        let global = stored_global(AgentKind::Codex, "Stop", true);
        let patch = stored_patch(AgentKind::Codex, "Stop", Some(true));
        let mut recreated_global = global.clone();
        let mut recreated_patch = patch.clone();
        recreated_global.version = 99;
        recreated_patch.version = 77;

        let first = resolve_stored_rule(&global, Some(&patch));
        let recreated = resolve_stored_rule(&recreated_global, Some(&recreated_patch));

        assert_eq!(first.id, global.id);
        assert_eq!(first.version, recreated.version);
        assert_eq!(first.version.len(), 64);
        assert!(first.version.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(first.version, first.version.to_ascii_lowercase());
    }

    #[test]
    fn effective_version_changes_with_behavior_or_project_identity() {
        let global = stored_global(AgentKind::Codex, "Stop", true);
        let patch = stored_patch(AgentKind::Codex, "Stop", Some(true));
        let mut changed_behavior = patch.clone();
        changed_behavior.patch.targets = Some(vec![target(4)]);
        let mut other_project = patch.clone();
        other_project.project_id = Uuid::from_u128(88);

        let baseline = resolve_stored_rule(&global, Some(&patch)).version;

        assert_ne!(
            baseline,
            resolve_stored_rule(&global, Some(&changed_behavior)).version
        );
        assert_ne!(
            baseline,
            resolve_stored_rule(&global, Some(&other_project)).version
        );
    }

    #[test]
    fn project_enablement_requires_hook_installation() {
        let selected = required_hook_selection(
            &[],
            &[stored_patch(AgentKind::Codex, "PostToolUse", Some(true))],
        );

        assert!(selected.contains(&(AgentKind::Codex, "PostToolUse".into())));
    }

    #[test]
    fn globally_and_project_enabled_events_are_selected_exactly_once() {
        let globals = [
            stored_global(AgentKind::Codex, "Stop", true),
            stored_global(AgentKind::Codex, "SessionEnd", false),
        ];
        let overrides = [
            stored_patch(AgentKind::Codex, "Stop", Some(true)),
            stored_patch(AgentKind::Codex, "Stop", Some(true)),
            stored_patch(AgentKind::Codex, "SessionEnd", Some(true)),
            stored_patch(AgentKind::ClaudeCode, "Notification", Some(true)),
        ];

        assert_eq!(
            required_hook_selection(&globals, &overrides)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                (AgentKind::ClaudeCode, "Notification".into()),
                (AgentKind::Codex, "SessionEnd".into()),
                (AgentKind::Codex, "Stop".into()),
            ]
        );
    }

    #[test]
    fn a_project_disablement_does_not_remove_a_globally_required_hook() {
        let globals = [stored_global(AgentKind::Codex, "Stop", true)];
        let overrides = [stored_patch(AgentKind::Codex, "Stop", Some(false))];

        assert_eq!(
            required_hook_selection(&globals, &overrides)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![(AgentKind::Codex, "Stop".into())]
        );
    }

    #[test]
    fn an_inheriting_patch_does_not_enable_a_disabled_hook() {
        let globals = [stored_global(AgentKind::Codex, "SessionEnd", false)];
        let overrides = [stored_patch(AgentKind::Codex, "SessionEnd", None)];

        assert!(required_hook_selection(&globals, &overrides).is_empty());
    }

    #[test]
    fn unknown_events_are_never_selected_for_hook_installation() {
        let globals = [
            stored_global(AgentKind::Codex, "Stop", true),
            stored_global(AgentKind::Codex, "ArbitraryHook", true),
        ];
        let overrides = [stored_patch(
            AgentKind::ClaudeCode,
            "UncataloguedEvent",
            Some(true),
        )];

        assert_eq!(
            required_hook_selection(&globals, &overrides)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![(AgentKind::Codex, "Stop".into())]
        );
    }

    #[test]
    fn validation_accepts_every_inclusive_limit() {
        let mut rule = default_rule(AgentKind::Codex, "Stop");
        rule.targets = (0..20).map(target).collect();
        rule.privacy.max_body_chars = 4_000;
        rule.delivery.mode = DeliveryMode::Aggregate { window_seconds: 10 };
        rule.delivery.cooldown_seconds = 86_400;
        rule.delivery.max_per_window = 1;
        rule.delivery.window_seconds = 86_400;
        rule.delivery.ttl_seconds = 1;
        rule.delivery.max_attempts = 1;
        rule.quiet_hours = Some(QuietHours {
            start_local: "00:00".into(),
            end_local: "23:59".into(),
            weekdays: vec![1, 7],
            bypass_at_or_above: None,
        });
        validate_rule(&rule).unwrap();

        rule.delivery.mode = DeliveryMode::Aggregate {
            window_seconds: 3_600,
        };
        rule.delivery.max_per_window = 100;
        rule.delivery.ttl_seconds = 86_400;
        rule.delivery.max_attempts = 10;

        validate_rule(&rule).unwrap();
    }

    #[test]
    fn validation_rejects_every_out_of_range_numeric_value() {
        let baseline = default_rule(AgentKind::Codex, "Stop");
        let mut invalid_rules = Vec::new();

        let mut rule = baseline.clone();
        rule.targets = (0..21).map(target).collect();
        invalid_rules.push(rule);
        for ttl in [0, 86_401] {
            let mut rule = baseline.clone();
            rule.delivery.ttl_seconds = ttl;
            invalid_rules.push(rule);
        }
        for max_attempts in [0, 11] {
            let mut rule = baseline.clone();
            rule.delivery.max_attempts = max_attempts;
            invalid_rules.push(rule);
        }
        for window_seconds in [9, 3_601] {
            let mut rule = baseline.clone();
            rule.delivery.mode = DeliveryMode::Aggregate { window_seconds };
            invalid_rules.push(rule);
        }
        let mut rule = baseline.clone();
        rule.delivery.cooldown_seconds = 86_401;
        invalid_rules.push(rule);
        let mut rule = baseline.clone();
        rule.delivery.window_seconds = 86_401;
        invalid_rules.push(rule);
        for max_per_window in [0, 101] {
            let mut rule = baseline.clone();
            rule.delivery.max_per_window = max_per_window;
            invalid_rules.push(rule);
        }
        let mut rule = baseline;
        rule.privacy.max_body_chars = 4_001;
        invalid_rules.push(rule);

        for rule in invalid_rules {
            assert_rule_invalid(&rule);
        }
    }

    #[test]
    fn validation_rejects_non_strict_times_and_invalid_weekdays() {
        for invalid_time in ["0:00", "00:0", " 00:00", "24:00", "23:60", "aa:bb"] {
            let mut rule = rule_with_quiet_hours(invalid_time, "08:00");
            assert_rule_invalid(&rule);
            rule.quiet_hours.as_mut().unwrap().start_local = "22:00".into();
            rule.quiet_hours.as_mut().unwrap().end_local = invalid_time.into();
            assert_rule_invalid(&rule);
        }

        for weekday in [0, 8] {
            let mut rule = rule_with_quiet_hours("22:00", "08:00");
            rule.quiet_hours.as_mut().unwrap().weekdays = vec![weekday];
            assert_rule_invalid(&rule);
        }
    }

    #[test]
    fn validation_rejects_unbounded_rule_collections_and_strings() {
        let baseline = default_rule(AgentKind::Codex, "Stop");
        let too_many = vec!["x".into(); 101];
        let too_long = vec!["x".repeat(257)];

        for filters in [
            FilterGroup {
                tool_names: too_many.clone(),
                ..FilterGroup::default()
            },
            FilterGroup {
                event_subtypes: too_many.clone(),
                ..FilterGroup::default()
            },
            FilterGroup {
                permission_modes: too_many.clone(),
                ..FilterGroup::default()
            },
            FilterGroup {
                models: too_many.clone(),
                ..FilterGroup::default()
            },
            FilterGroup {
                statuses: too_many.clone(),
                ..FilterGroup::default()
            },
            FilterGroup {
                tool_names: too_long.clone(),
                ..FilterGroup::default()
            },
            FilterGroup {
                event_subtypes: too_long.clone(),
                ..FilterGroup::default()
            },
            FilterGroup {
                permission_modes: too_long.clone(),
                ..FilterGroup::default()
            },
            FilterGroup {
                models: too_long.clone(),
                ..FilterGroup::default()
            },
            FilterGroup {
                statuses: too_long.clone(),
                ..FilterGroup::default()
            },
        ] {
            let mut rule = baseline.clone();
            rule.filters = filters;
            assert_rule_invalid(&rule);
        }

        let mut rule = baseline.clone();
        rule.privacy.allowed_sensitive_fields = too_many.clone();
        assert_rule_invalid(&rule);
        rule.privacy.allowed_sensitive_fields = too_long.clone();
        assert_rule_invalid(&rule);
    }

    #[test]
    fn validation_accepts_task_five_inclusive_privacy_bounds() {
        let mut rule = default_rule(AgentKind::Codex, "Stop");
        rule.privacy.extra_redaction_patterns = vec!["密".repeat(512); 32];
        rule.targets = vec![TargetConfig {
            channel_id: Uuid::from_u128(99),
            template: Some("x".repeat(16 * 1024)),
        }];

        assert!(validate_rule(&rule).is_ok());
    }

    #[test]
    fn validation_rejects_task_five_exclusive_privacy_bounds() {
        let mut rule = default_rule(AgentKind::Codex, "Stop");
        rule.privacy.extra_redaction_patterns = vec!["safe".into(); 33];
        assert_rule_invalid(&rule);

        rule.privacy.extra_redaction_patterns = vec!["密".repeat(513)];
        assert_rule_invalid(&rule);

        rule.privacy.extra_redaction_patterns.clear();
        rule.targets = vec![TargetConfig {
            channel_id: Uuid::from_u128(99),
            template: Some("x".repeat(16 * 1024 + 1)),
        }];
        assert_rule_invalid(&rule);
    }

    #[test]
    fn defaults_enable_only_the_declared_agent_events() {
        for event in ["PermissionRequest", "Notification", "Stop", "StopFailure"] {
            assert!(default_rule(AgentKind::ClaudeCode, event).enabled);
        }
        for event in ["PermissionRequest", "Stop"] {
            assert!(default_rule(AgentKind::Codex, event).enabled);
        }
        for (agent, event) in [
            (AgentKind::ClaudeCode, "SessionEnd"),
            (AgentKind::ClaudeCode, "PostToolUse"),
            (AgentKind::Codex, "SessionEnd"),
            (AgentKind::Codex, "PostToolUse"),
            (AgentKind::Codex, "Notification"),
        ] {
            assert!(!default_rule(agent, event).enabled);
        }
    }

    #[test]
    fn defaults_apply_ttl_retries_and_private_summary_modes() {
        let permission = default_rule(AgentKind::Codex, "PermissionRequest");
        let completion = default_rule(AgentKind::Codex, "Stop");
        let failure = default_rule(AgentKind::ClaudeCode, "StopFailure");
        let metadata = default_rule(AgentKind::Codex, "SessionEnd");

        assert_eq!(permission.delivery.ttl_seconds, 600);
        assert_eq!(completion.delivery.ttl_seconds, 1_800);
        assert_eq!(completion.delivery.max_attempts, 5);
        assert_eq!(completion.privacy.summary_mode, SummaryMode::NativeSummary);
        assert_eq!(failure.privacy.summary_mode, SummaryMode::NativeSummary);
        assert_eq!(completion.privacy.max_body_chars, 500);
        assert_eq!(metadata.privacy.summary_mode, SummaryMode::MetadataOnly);
        assert_eq!(metadata.privacy.max_body_chars, 0);
        for rule in [permission, completion, failure, metadata] {
            assert!(rule.privacy.allowed_sensitive_fields.is_empty());
            assert!(rule.privacy.extra_redaction_patterns.is_empty());
        }
    }

    fn enabled_rule_with_targets(targets: Vec<TargetConfig>) -> RuleConfig {
        RuleConfig {
            targets,
            ..default_rule(AgentKind::Codex, "Stop")
        }
    }

    fn rule_with_quiet_hours(start: &str, end: &str) -> RuleConfig {
        RuleConfig {
            quiet_hours: Some(QuietHours {
                start_local: start.into(),
                end_local: end.into(),
                weekdays: vec![1, 2, 3, 4, 5, 6, 7],
                bypass_at_or_above: None,
            }),
            ..default_rule(AgentKind::Codex, "Stop")
        }
    }

    fn target(value: u128) -> TargetConfig {
        TargetConfig {
            channel_id: Uuid::from_u128(value + 1),
            template: None,
        }
    }

    fn stored_global(agent: AgentKind, event: &str, enabled: bool) -> StoredGlobalRule {
        StoredGlobalRule {
            id: Uuid::from_u128(1),
            agent,
            source_event: event.into(),
            version: 1,
            config: RuleConfig {
                enabled,
                ..default_rule(agent, event)
            },
        }
    }

    fn stored_patch(agent: AgentKind, event: &str, enabled: Option<bool>) -> StoredRulePatch {
        StoredRulePatch {
            project_id: Uuid::from_u128(2),
            agent,
            source_event: event.into(),
            version: 1,
            patch: RulePatch {
                enabled,
                ..RulePatch::default()
            },
        }
    }

    fn assert_rule_invalid(rule: &RuleConfig) {
        let error = validate_rule(rule).unwrap_err();
        assert_eq!(error.domain, ErrorDomain::Configuration);
        assert_eq!(error.code, "rule_invalid");
    }
}
